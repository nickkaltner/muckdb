//! Session export/import. A `.muckdb` file is a zip bundling a session's JSON,
//! full snapshots of every database its tiles reference, and the column-format
//! registry entries for those databases — enough to rebuild the dashboard on
//! another machine (`muckdb session export/import`, and the web UI's
//! export/import buttons).

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::formats;
use crate::session::{Session, Tile};
use crate::store;

/// Newest archive layout this build writes; imports refuse anything newer.
const FORMAT_VERSION: u32 = 1;

/// One bundled database in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestDb {
    /// `store::db_id` of the original path (what format entries are keyed by).
    pub id: String,
    pub original_path: String,
    /// Archive-relative file, e.g. `dbs/<id>.duckdb`.
    pub file: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub session: String,
    #[serde(default)]
    pub title: Option<String>,
    pub muckdb_version: String,
    pub exported: u64,
    pub dbs: Vec<ManifestDb>,
    /// Format-registry entries for the bundled dbs, re-keyed on import.
    #[serde(default)]
    pub formats: Vec<formats::Entry>,
}

/// Distinct database paths referenced by the session's data tiles, in
/// first-use order.
fn tile_dbs(session: &Session) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for t in &session.tiles {
        if let Tile::View { db, .. } = t
            && !seen.contains(db)
        {
            seen.push(db.clone());
        }
    }
    seen
}

/// Snapshot `src` into `dst`: a clean, checkpointed, compacted copy via the
/// duckdb CLI (`ATTACH` both, `COPY FROM DATABASE`), which never captures a
/// mid-write WAL. Falls back to a raw byte copy (+ `.wal`) if the CLI copy
/// fails (e.g. an old duckdb without COPY FROM DATABASE).
fn snapshot_db(src: &str, dst: &Path) -> Result<()> {
    let _ = fs::remove_file(dst);
    let esc = |s: &str| s.replace('\'', "''");
    let sql = format!(
        "ATTACH '{}' AS snap_in (READ_ONLY); ATTACH '{}' AS snap_out; COPY FROM DATABASE snap_in TO snap_out;",
        esc(src),
        esc(&dst.display().to_string())
    );
    let out = std::process::Command::new("duckdb")
        .arg("-c")
        .arg(&sql)
        .output()
        .context("failed to run `duckdb` — is it installed and on PATH?")?;
    if out.status.success() && dst.exists() {
        return Ok(());
    }
    let _ = fs::remove_file(dst);
    fs::copy(src, dst).with_context(|| format!("copying {src} to {dst:?}"))?;
    let wal = format!("{src}.wal");
    if Path::new(&wal).exists() {
        let _ = fs::copy(&wal, dst.with_extension("duckdb.wal"));
    }
    Ok(())
}

/// Build the `.muckdb` archive for `session` at `out`; returns the manifest.
pub fn build_archive(
    session: &Session,
    format_entries: Vec<formats::Entry>,
    out: &Path,
) -> Result<Manifest> {
    let tmp = std::env::temp_dir().join(format!(
        "muckdb-export-{}-{}",
        std::process::id(),
        store::now_millis()
    ));
    fs::create_dir_all(&tmp).with_context(|| format!("creating {tmp:?}"))?;
    let result = build_archive_in(session, format_entries, out, &tmp);
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn build_archive_in(
    session: &Session,
    format_entries: Vec<formats::Entry>,
    out: &Path,
    tmp: &Path,
) -> Result<Manifest> {
    use zip::write::SimpleFileOptions;
    let mut manifest = Manifest {
        format: FORMAT_VERSION,
        session: session.id.clone(),
        title: session.title.clone(),
        muckdb_version: env!("CARGO_PKG_VERSION").to_string(),
        exported: store::now_millis(),
        dbs: Vec::new(),
        formats: format_entries,
    };
    let file = fs::File::create(out).with_context(|| format!("creating {out:?}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    for db in tile_dbs(session) {
        if !Path::new(&db).exists() {
            bail!("database not found: {db} (a tile references it)");
        }
        let id = store::db_id(&db);
        let file_name = format!("dbs/{id}.duckdb");
        let snap = tmp.join(format!("{id}.duckdb"));
        snapshot_db(&db, &snap)?;
        zip.start_file(&file_name, opts)?;
        std::io::copy(&mut fs::File::open(&snap)?, &mut zip)?;
        // Present only when the snapshot fell back to a raw byte copy.
        let wal = snap.with_extension("duckdb.wal");
        if wal.exists() {
            zip.start_file(format!("{file_name}.wal"), opts)?;
            std::io::copy(&mut fs::File::open(&wal)?, &mut zip)?;
        }
        manifest.dbs.push(ManifestDb {
            id,
            original_path: db.clone(),
            file: file_name,
        });
    }
    zip.start_file("session.json", opts)?;
    zip.write_all(serde_json::to_string_pretty(session)?.as_bytes())?;
    zip.start_file("manifest.json", opts)?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    zip.finish()?;
    Ok(manifest)
}

/// The outcome of unpacking an archive: the rewritten session (collision-free
/// id, tile paths pointing at the imported copies), the re-keyed format
/// entries, and where the dbs landed.
#[derive(Debug)]
pub struct Imported {
    pub session: Session,
    pub formats: Vec<formats::Entry>,
    pub dbs: Vec<PathBuf>,
}

/// First id not in `existing`: `want`, else `want-2`, `want-3`, …
fn free_id(want: &str, existing: &[String]) -> String {
    if !existing.iter().any(|e| e == want) {
        return want.to_string();
    }
    (2..)
        .map(|n| format!("{want}-{n}"))
        .find(|c| !existing.iter().any(|e| e == c))
        .expect("unbounded")
}

fn read_zip_string<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Result<String> {
    let mut f = zip
        .by_name(name)
        .with_context(|| format!("archive has no {name} — not a .muckdb export?"))?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

fn extract_zip_file<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    dest: &Path,
) -> Result<()> {
    let mut f = zip
        .by_name(name)
        .with_context(|| format!("archive is missing {name}"))?;
    let mut out = fs::File::create(dest).with_context(|| format!("creating {dest:?}"))?;
    std::io::copy(&mut f, &mut out)?;
    Ok(())
}

/// Unpack a `.muckdb` archive. Databases are extracted to
/// `<imports_root>/<final-id>/`; tile db paths and format-entry keys are
/// rewritten to the new locations; a session-id collision against
/// `existing_ids` gets a numeric suffix. Touches nothing outside
/// `imports_root` — the caller persists the session/formats/ledger.
pub fn import_archive(
    bytes: &[u8],
    imports_root: &Path,
    existing_ids: &[String],
) -> Result<Imported> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("reading .muckdb zip")?;
    let manifest: Manifest = serde_json::from_str(&read_zip_string(&mut zip, "manifest.json")?)
        .context("parsing manifest.json")?;
    if manifest.format > FORMAT_VERSION {
        bail!(
            "archive is format v{} but this muckdb reads up to v{FORMAT_VERSION} — upgrade muckdb",
            manifest.format
        );
    }
    let mut session: Session = serde_json::from_str(&read_zip_string(&mut zip, "session.json")?)
        .context("parsing session.json")?;
    session.id = free_id(&session.id, existing_ids);

    let dest_dir = imports_root.join(&session.id);
    fs::create_dir_all(&dest_dir).with_context(|| format!("creating {dest_dir:?}"))?;
    let mut path_map: BTreeMap<String, String> = BTreeMap::new();
    let mut dbs = Vec::new();
    for db in &manifest.dbs {
        let base = Path::new(&db.file)
            .file_name()
            .context("bad db file name in manifest")?
            .to_owned();
        let dest = dest_dir.join(&base);
        extract_zip_file(&mut zip, &db.file, &dest)?;
        // A companion WAL exists only for raw-copy fallback snapshots.
        let wal_name = format!("{}.wal", db.file);
        if zip.by_name(&wal_name).is_ok() {
            let wal_dest = dest_dir.join(format!("{}.wal", base.to_string_lossy()));
            extract_zip_file(&mut zip, &wal_name, &wal_dest)?;
        }
        path_map.insert(db.original_path.clone(), dest.display().to_string());
        dbs.push(dest);
    }

    // Point every tile at its imported copy.
    for t in &mut session.tiles {
        if let Tile::View { db, .. } = t
            && let Some(new) = path_map.get(db)
        {
            *db = new.clone();
        }
    }

    // Re-key format entries from the original paths' ids to the imported ones;
    // drop entries for dbs that weren't bundled.
    let id_map: BTreeMap<String, String> = manifest
        .dbs
        .iter()
        .filter_map(|d| {
            path_map
                .get(&d.original_path)
                .map(|new| (d.id.clone(), store::db_id(new)))
        })
        .collect();
    let mut fmts = manifest.formats;
    fmts.retain_mut(|e| match id_map.get(&e.db) {
        Some(new) => {
            e.db = new.clone();
            true
        }
        None => false,
    });

    Ok(Imported {
        session,
        formats: fmts,
        dbs,
    })
}

/// `muckdb session export`: write the archive (default `./<id>.muckdb`).
pub fn export_session(id: &str, out: Option<PathBuf>) -> Result<PathBuf> {
    let session = crate::session::load(id)?.with_context(|| format!("no such session '{id}'"))?;
    let out = out.unwrap_or_else(|| PathBuf::from(format!("{id}.muckdb")));
    let ids: Vec<String> = tile_dbs(&session).iter().map(|p| store::db_id(p)).collect();
    let fmts = formats::entries_for_db_ids(&ids)?;
    build_archive(&session, fmts, &out)?;
    Ok(out)
}

/// The daemon's export: build to a temp file, return (session id, zip bytes).
/// The caller builds the download filename as `<id>.muckdb`.
pub fn export_session_bytes(id: &str) -> Result<(String, Vec<u8>)> {
    let tmp = std::env::temp_dir().join(format!(
        "muckdb-export-{}-{}.muckdb",
        std::process::id(),
        store::now_millis()
    ));
    let result = export_session(id, Some(tmp.clone()))
        .and_then(|p| fs::read(&p).with_context(|| format!("reading {p:?}")));
    let _ = fs::remove_file(&tmp);
    Ok((id.to_string(), result?))
}

/// Full import: unpack under `<data-dir>/imports/`, save the session, merge
/// the format entries, and append a completed ledger invocation per imported
/// db so they appear in the databases tab and `muckdb ls databases`.
pub fn import_and_install(bytes: &[u8]) -> Result<Imported> {
    let existing: Vec<String> = crate::session::list()?.into_iter().map(|s| s.id).collect();
    let root = crate::paths::data_dir()?.join("imports");
    let imported = import_archive(bytes, &root, &existing)?;
    crate::session::save(&imported.session)?;
    formats::merge_entries(imported.formats.clone())?;
    for (i, db) in imported.dbs.iter().enumerate() {
        record_import(db, &imported.session.id, i as u64)?;
    }
    Ok(imported)
}

/// One completed ledger invocation for an imported db.
fn record_import(db: &Path, session_id: &str, seq: u64) -> Result<()> {
    let id = store::now_millis() + seq;
    let mk = |phase, exit_code| store::Record {
        id,
        ts: store::now_millis(),
        cwd: std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        args: vec!["session".into(), "import".into()],
        db_path: Some(db.display().to_string()),
        phase,
        exit_code,
        session: Some(session_id.to_string()),
    };
    store::append(&mk(store::Phase::Start, None))?;
    store::append(&mk(store::Phase::End, Some(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Chart, Session, Tile};

    /// The archive/import tests need the `duckdb` CLI (same convention as the
    /// introspect stats tests) — skip gracefully where it isn't installed.
    fn duckdb_ok() -> bool {
        std::process::Command::new("duckdb")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run_sql(db: &std::path::Path, sql: &str) {
        let out = std::process::Command::new("duckdb")
            .arg(db)
            .arg("-c")
            .arg(sql)
            .output()
            .expect("duckdb runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("muckdb-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample_session(db: &std::path::Path) -> Session {
        Session {
            id: "exp-test".into(),
            title: Some("Export test".into()),
            claude_session: None,
            created: 1,
            updated: 2,
            tiles: vec![
                Tile::Markdown {
                    name: "intro".into(),
                    title: None,
                    markdown: "# hi".into(),
                    trashed: false,
                },
                Tile::View {
                    name: "chart".into(),
                    title: None,
                    db: db.display().to_string(),
                    view: Some("v_counts".into()),
                    sql: None,
                    chart: Box::new(Chart {
                        kind: "bar".into(),
                        x: Some("k".into()),
                        y: vec!["n".into()],
                        xlabel: None,
                        ylabel: None,
                        bars: None,
                        targets: vec![],
                        thresholds: vec![],
                        events: vec![],
                    }),
                    caption: Some("c".into()),
                    trashed: false,
                },
            ],
        }
    }

    #[test]
    fn build_archive_bundles_manifest_session_and_db_snapshot() {
        if !duckdb_ok() {
            eprintln!("skipping build_archive_bundles_manifest_session_and_db_snapshot: no duckdb");
            return;
        }
        let dir = temp_dir("build");
        let db = dir.join("data.duckdb");
        run_sql(
            &db,
            "CREATE TABLE t(k TEXT, n INT); INSERT INTO t VALUES ('a', 1), ('b', 2); CREATE VIEW v_counts AS SELECT k, n FROM t;",
        );
        let session = sample_session(&db);
        let out = dir.join("exp-test.muckdb");
        let manifest = build_archive(&session, vec![], &out).unwrap();

        assert_eq!(manifest.format, FORMAT_VERSION);
        assert_eq!(manifest.session, "exp-test");
        assert_eq!(manifest.dbs.len(), 1);
        assert_eq!(manifest.dbs[0].original_path, db.display().to_string());

        let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        assert!(zip.by_name("manifest.json").is_ok());
        assert!(zip.by_name("session.json").is_ok());
        assert!(zip.by_name(&manifest.dbs[0].file).is_ok());
    }

    #[test]
    fn snapshot_is_a_working_duckdb_with_the_view() {
        if !duckdb_ok() {
            eprintln!("skipping snapshot_is_a_working_duckdb_with_the_view: no duckdb");
            return;
        }
        let dir = temp_dir("snap");
        let db = dir.join("src.duckdb");
        run_sql(
            &db,
            "CREATE TABLE t(k TEXT); INSERT INTO t VALUES ('x'); CREATE VIEW v_counts AS SELECT k FROM t;",
        );
        let snap = dir.join("snap.duckdb");
        snapshot_db(&db.display().to_string(), &snap).unwrap();
        let rows =
            crate::introspect::query_json(&snap.display().to_string(), "SELECT k FROM v_counts")
                .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn free_id_suffixes_on_collision() {
        assert_eq!(free_id("a", &[]), "a");
        assert_eq!(free_id("a", &["a".into()]), "a-2");
        assert_eq!(free_id("a", &["a".into(), "a-2".into()]), "a-3");
    }

    #[test]
    fn import_round_trip_rewrites_paths_and_rekeys_formats() {
        if !duckdb_ok() {
            eprintln!("skipping import_round_trip_rewrites_paths_and_rekeys_formats: no duckdb");
            return;
        }
        let dir = temp_dir("import");
        let db = dir.join("data.duckdb");
        run_sql(
            &db,
            "CREATE TABLE t(k TEXT, n INT); INSERT INTO t VALUES ('a', 1); CREATE VIEW v_counts AS SELECT k, n FROM t;",
        );
        let session = sample_session(&db);
        let fmt = formats::Entry {
            db: crate::store::db_id(&db.display().to_string()),
            table: None,
            column: "n".into(),
            format: formats::Format {
                prefix: Some("$".into()),
                ..Default::default()
            },
        };
        let out = dir.join("exp-test.muckdb");
        build_archive(&session, vec![fmt], &out).unwrap();

        let bytes = std::fs::read(&out).unwrap();
        let root = dir.join("imports");
        let imported = import_archive(&bytes, &root, &["exp-test".into()]).unwrap();

        // Collision → suffixed id; dbs land under imports/<final-id>/.
        assert_eq!(imported.session.id, "exp-test-2");
        assert_eq!(imported.dbs.len(), 1);
        assert!(imported.dbs[0].starts_with(root.join("exp-test-2")));
        assert!(imported.dbs[0].exists());

        // Tile db paths rewritten to the imported copies.
        let Tile::View { db: tile_db, .. } = &imported.session.tiles[1] else {
            panic!("view tile")
        };
        assert_eq!(tile_db, &imported.dbs[0].display().to_string());

        // Format entries re-keyed to the imported path's id.
        assert_eq!(imported.formats.len(), 1);
        assert_eq!(imported.formats[0].db, crate::store::db_id(tile_db));

        // The imported snapshot actually answers queries.
        let rows = crate::introspect::query_json(tile_db, "SELECT n FROM v_counts").unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn import_refuses_non_archives() {
        let root = temp_dir("badimport");
        assert!(import_archive(b"not a zip", &root, &[]).is_err());
    }

    #[test]
    fn import_refuses_newer_format() {
        if !duckdb_ok() {
            eprintln!("skipping import_refuses_newer_format: no duckdb");
            return;
        }
        let dir = temp_dir("newformat");
        let db = dir.join("data.duckdb");
        run_sql(
            &db,
            "CREATE TABLE t(k TEXT); CREATE VIEW v_counts AS SELECT k FROM t;",
        );
        let out = dir.join("exp-test.muckdb");
        build_archive(&sample_session(&db), vec![], &out).unwrap();
        // Rewrite the archive with a bumped manifest format version.
        let bytes = std::fs::read(&out).unwrap();
        let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut manifest = String::new();
        std::io::Read::read_to_string(&mut zin.by_name("manifest.json").unwrap(), &mut manifest)
            .unwrap();
        let manifest = manifest.replace("\"format\": 1", "\"format\": 99");
        let mut zout = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).unwrap();
            if f.name() == "manifest.json" {
                continue;
            }
            zout.start_file(f.name().to_string(), opts).unwrap();
            std::io::copy(&mut f, &mut zout).unwrap();
        }
        zout.start_file("manifest.json", opts).unwrap();
        std::io::Write::write_all(&mut zout, manifest.as_bytes()).unwrap();
        zout.finish().unwrap();

        let err = import_archive(&std::fs::read(&out).unwrap(), &dir.join("imports"), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("upgrade muckdb"), "{err}");
    }

    #[test]
    fn tile_dbs_dedupes_and_skips_markdown() {
        let dir = temp_dir("dbs");
        let db = dir.join("d.duckdb");
        let mut s = sample_session(&db);
        s.tiles.push(s.tiles[1].clone()); // second tile on the same db
        assert_eq!(tile_dbs(&s), vec![db.display().to_string()]);
    }
}
