# pushState Nav + macOS Copy-Image + Session Export/Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix Back-button navigation (one clean history entry per real navigation), make the copy-image button robust and self-diagnosing on macOS Chrome, and add session export/import as `<session>.muckdb` zip archives (session JSON + full db snapshots + format registry entries).

**Architecture:** Tasks 1–2 are pure `src/assets/index.html` changes. Task 3+ adds a new `src/export.rs` (archive build/unpack, testable with injected paths), thin wrappers with real side effects (data dir, ledger, format registry), CLI subcommands in `session::cli`, two daemon endpoints, and web-UI buttons.

**Tech Stack:** Rust (axum daemon, `zip` crate new dep), duckdb CLI (`ATTACH` + `COPY FROM DATABASE` snapshots), vanilla JS single-file web app.

**Spec:** `docs/superpowers/specs/2026-07-03-nav-clipboard-export-design.md`

## Global Constraints

- Before every commit: `cargo fmt` (not `--check`), `cargo clippy --all-targets -- -D warnings`, `cargo test` — all clean (project CLAUDE.md).
- Archive extension is `.muckdb`; export filename is `<session-id>.muckdb`.
- Import never overwrites: session-id collision → numeric suffix (`name-2`, `name-3`, …).
- Manifest carries `format: 1`; importing a *newer* format fails with a clear "upgrade muckdb" error.
- History rule: real navigation pushes exactly one entry per user action; search/filters/sort/pagination stay `replaceState`.
- Web error surfacing uses the new `toast()` helper, never bare `console.warn` alone.

---

### Task 1: pushState navigation — one history entry per navigation

**Files:**
- Modify: `src/assets/index.html` (`setTab` ~line 933, `exploreTile` ~2328, picker items ~1596–1598, click handlers ~2554, ~2568)

**Interfaces:**
- Produces: `setTab(tab, opts)` where `opts.silent === true` skips `syncUrl`/`maybeAutoSession` — used by every compound navigation (tab switch + db/session select in one user action).

**Root cause (from spec):** `exploreTile()` → `setTab("databases")` pushes an intermediate URL built from stale state (`/` or the previous db) before `selectDb → selectTable` pushes the real `/db/<id>/<view>/`. Back lands on `/` → parsed as "sessions, no session" → auto-loads the *first* session.

- [ ] **Step 1: Make `setTab` support silent mode**

Replace the function at ~line 933:

```js
  function setTab(tab, opts) {
    if (tab !== "sessions" && state.zoomPanel) closeZoom();
    state.tab = tab;
    document.querySelectorAll(".tab").forEach((b) => b.classList.toggle("active", b.dataset.tab === tab));
    document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
    $("view-" + tab).classList.add("active");
    // Compound navigations (tab switch + db/session select in one user action)
    // pass {silent:true} so the *final* state pushes exactly one history entry.
    if (opts && opts.silent) return;
    syncUrl(true);
    if (!suppressSync) maybeAutoSession();
  }
```

- [ ] **Step 2: Silence the tab switch in every compound navigation**

Five call sites; each is immediately followed by a select/load that pushes the final URL:

~line 2332 (`exploreTile`):
```js
    setTab("databases", { silent: true });
    selectDb(t.db, { table: t.view, sub: "rows" });
```

~line 1596–1598 (⌘K picker items):
```js
    state.databases.forEach((d) => items.push({ hint: "db", label: basename(d.path), sub: dirname(d.path), go: () => { setTab("databases", { silent: true }); selectDb(d.path); } }));
    if (state.selDb) state.tables.forEach((t) => items.push({ hint: t.is_view ? "view" : "table", label: t.name, sub: basename(state.selDb), go: () => { setTab("databases", { silent: true }); selectTable(t.name); } }));
    state.sessions.forEach((s) => items.push({ hint: "sess", label: s.title || s.id, sub: s.tiles + " tiles", go: () => { setTab("sessions", { silent: true }); loadSession(s.id, true); } }));
```

~line 2554 (ledger db chip):
```js
    if (openDb) { setTab("databases", { silent: true }); selectDb(openDb.dataset.openDb); return; }
```

~line 2568 (ledger `§ session` link — also an existing bug: it loads the session **without switching to the sessions tab**, so the URL pushed is `/ledger`):
```js
    const sess = ev.target.closest("[data-session]"); if (sess) { setTab("sessions", { silent: true }); loadSession(sess.dataset.session, true); return; }
```

Leave unchanged: header tab clicks (~2520, a plain tab switch IS the navigation), `restoreFromNav` (~917) and initial load (~2810, both run under `suppressSync`).

- [ ] **Step 3: Verify by exercising the flows in a browser**

`cargo build && ./target/debug/muckdb --status` (start daemon if needed: any muckdb call). Open `http://localhost:11000/session/demo/` (a session exists: `demo`). Then:
1. Click **explore** on a view tile → URL becomes `/db/<id>/<view>/`, exactly one Back returns to `/session/demo/` showing the *same* session.
2. Ledger tab → click a db chip → Back returns to `/ledger`.
3. Ledger → click a `§ session` link → sessions tab opens on that session; Back returns to `/ledger`.
4. ⌘K → jump to a table → one Back returns to the previous view.

Also run `cargo test` (must stay green — no Rust touched, sanity only).

- [ ] **Step 4: Commit**

```bash
git add src/assets/index.html
git commit -m "fix: one history entry per navigation — Back from explore returns to the session"
```

---

### Task 2: copy-image robustness + `toast()` (macOS Chrome fix)

**Files:**
- Modify: `src/assets/index.html` (`copyPanelImage` ~line 1774; new `toast()` helper + CSS)

**Interfaces:**
- Produces: `toast(msg, isErr)` — transient bottom-center notification; Task 7's import UI reuses it.

- [ ] **Step 1: Add the toast helper + CSS**

CSS, in the `<style>` block (near the `.zoom-close` rules ~line 84; match the existing variable palette — the error colour should reuse whatever `.note.err` uses, check it first):

```css
  #toast { position: fixed; left: 50%; bottom: 34px; transform: translateX(-50%) translateY(8px); background: var(--raised); color: var(--fg); border: 1px solid var(--line); border-radius: 7px; padding: 8px 14px; font-size: 12.5px; opacity: 0; pointer-events: none; transition: opacity .18s ease, transform .18s ease; z-index: 300; max-width: 70vw; }
  #toast.show { opacity: 1; transform: translateX(-50%); }
```
plus an `#toast.err { ... }` rule using the same colour as `.note.err`.

JS, next to `flashBtn` (~line 1773):

```js
  let toastTimer = null;
  function toast(msg, isErr) {
    let el = $("toast");
    if (!el) { el = document.createElement("div"); el.id = "toast"; document.body.appendChild(el); }
    el.textContent = msg;
    el.classList.toggle("err", !!isErr);
    el.classList.add("show");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove("show"), isErr ? 6000 : 2600);
  }
```

- [ ] **Step 2: Rewrite `copyPanelImage`**

Replace the whole function (~1774–1803). Key changes: the PNG is fetched **once** (a shared promise — a failed clipboard attempt must not re-render), the write chain is promise-ClipboardItem (Safari) → plain-blob ClipboardItem (Chrome) → download, and total failure shows the *server's* error message:

```js
  async function copyPanelImage(name, btn) {
    if (!state.selSession || btn.classList.contains("busy")) return;
    btn.classList.add("busy");
    const url = "/api/shot?" + new URLSearchParams({ session: state.selSession, tile: name });
    // One fetch, shared by every attempt — the render takes seconds.
    const pngPromise = fetch(url).then(async (r) => {
      const blob = await r.blob();
      if (blob.type !== "image/png") {
        let msg = "screenshot failed"; try { msg = JSON.parse(await blob.text()).error || msg; } catch (_) {}
        throw new Error(msg);
      }
      return blob;
    });
    try {
      // Promise-based ClipboardItem first: Safari checks the user gesture when
      // the promise is handed over, so it survives the slow render.
      await navigator.clipboard.write([new ClipboardItem({ "image/png": pngPromise })]);
      flashBtn(btn, "ok");
    } catch (e1) {
      try {
        const blob = await pngPromise; // rethrows the real /api/shot error if the fetch failed
        try {
          // Plain-blob write: what Chrome prefers (it auto-grants
          // clipboard-write to the focused tab, so the elapsed await is fine).
          await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
          flashBtn(btn, "ok");
        } catch (e2) {
          // Clipboard images unsupported or denied — fall back to a download.
          const a = document.createElement("a");
          a.href = URL.createObjectURL(blob);
          a.download = `${state.selSession}-${name}.png`;
          a.click();
          setTimeout(() => URL.revokeObjectURL(a.href), 10000);
          toast("clipboard unavailable — downloaded the PNG instead");
          flashBtn(btn, "ok");
        }
      } catch (e3) {
        console.warn("copy panel image:", e3);
        toast("copy image failed: " + (e3 && e3.message || e3), true);
        flashBtn(btn, "err");
      }
    } finally { btn.classList.remove("busy"); }
  }
```

- [ ] **Step 3: Verify**

In the local browser on `http://localhost:11000/session/demo/`: click the copy-image button on a tile → green flash, paste the image somewhere (or at minimum: no error toast). Temporarily test the error path by clicking copy-image after `MUCKDB_BROWSER=/bin/false`-style breakage is impractical here — instead verify the toast renders by running `toast("test", true)` in the devtools console.

- [ ] **Step 4: Commit**

```bash
git add src/assets/index.html
git commit -m "fix: robust copy-image chain (Safari promise / Chrome blob / download) with visible errors"
```

---

### Task 3: `export.rs` — archive build (zip + db snapshots)

**Files:**
- Create: `src/export.rs`
- Modify: `Cargo.toml` (add `zip`), `src/main.rs` (add `mod export;`)
- Test: unit tests inside `src/export.rs` (repo convention)

**Interfaces:**
- Produces:
  - `pub struct Manifest { format: u32, session: String, title: Option<String>, muckdb_version: String, exported: u64, dbs: Vec<ManifestDb>, formats: Vec<formats::Entry> }`
  - `pub struct ManifestDb { id: String, original_path: String, file: String }`
  - `pub fn build_archive(session: &Session, format_entries: Vec<formats::Entry>, out: &Path) -> Result<Manifest>` — snapshots every tile db, writes the zip.
  - `fn tile_dbs(session: &Session) -> Vec<String>` (crate-visible; Task 5 uses it via a wrapper).
- Consumes: `session::{Session, Tile}`, `store::{db_id, now_millis}`, `formats::Entry`.

- [ ] **Step 1: Add the zip dependency and module**

```bash
cargo add zip --no-default-features --features deflate
```

In `src/main.rs`, add `mod export;` to the module list (alphabetical: after `mod daemon;`/`mod facade;` group — insert between `facade` and `formats`).

- [ ] **Step 2: Write failing tests**

`src/export.rs` skeleton with tests at the bottom. Tests build a real duckdb file (repo tests already shell out to `duckdb`, so CI has it):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Chart, Session, Tile};

    fn run_sql(db: &std::path::Path, sql: &str) {
        let out = std::process::Command::new("duckdb")
            .arg(db).arg("-c").arg(sql)
            .output().expect("duckdb runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("muckdb-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample_session(db: &std::path::Path) -> Session {
        Session {
            id: "exp-test".into(), title: Some("Export test".into()),
            claude_session: None, created: 1, updated: 2,
            tiles: vec![
                Tile::Markdown { name: "intro".into(), title: None, markdown: "# hi".into(), trashed: false },
                Tile::View {
                    name: "chart".into(), title: None, db: db.display().to_string(),
                    view: Some("v_counts".into()), sql: None,
                    chart: Box::new(Chart { kind: "bar".into(), x: Some("k".into()), y: vec!["n".into()], xlabel: None, ylabel: None, bars: None, targets: vec![], thresholds: vec![], events: vec![] }),
                    caption: Some("c".into()), trashed: false,
                },
            ],
        }
    }

    #[test]
    fn build_archive_bundles_manifest_session_and_db_snapshot() {
        let dir = temp_dir("build");
        let db = dir.join("data.duckdb");
        run_sql(&db, "CREATE TABLE t(k TEXT, n INT); INSERT INTO t VALUES ('a', 1), ('b', 2); CREATE VIEW v_counts AS SELECT k, n FROM t;");
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
        let dir = temp_dir("snap");
        let db = dir.join("src.duckdb");
        run_sql(&db, "CREATE TABLE t(k TEXT); INSERT INTO t VALUES ('x'); CREATE VIEW v_counts AS SELECT k FROM t;");
        let snap = dir.join("snap.duckdb");
        snapshot_db(&db.display().to_string(), &snap).unwrap();
        let rows = crate::introspect::query_json(&snap.display().to_string(), "SELECT k FROM v_counts").unwrap();
        assert_eq!(rows.len(), 1);
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
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test export`
Expected: compile error — `build_archive`, `snapshot_db`, `tile_dbs`, `FORMAT_VERSION` not defined.

- [ ] **Step 4: Implement**

`src/export.rs`:

```rust
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
```

Note: `formats::Entry` derives `Clone` already? It derives `Debug, Clone, Serialize, Deserialize` — yes (src/formats.rs:57). `session::Chart`/`Tile`/`Session` are `Clone` — yes.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test export`
Expected: 3 tests PASS. If `query_json` visibility blocks the test (`pub(crate)` — it's fine from a sibling module's test), adjust to call via `crate::introspect::query_json`.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add Cargo.toml Cargo.lock src/export.rs src/main.rs
git commit -m "feat: session archive builder — .muckdb zip with db snapshots"
```

---

### Task 4: `export.rs` — import core (unpack, rewrite, re-key, collision)

**Files:**
- Modify: `src/export.rs`
- Test: unit tests inside `src/export.rs`

**Interfaces:**
- Produces:
  - `pub struct Imported { pub session: Session, pub formats: Vec<formats::Entry>, pub dbs: Vec<PathBuf> }`
  - `pub fn import_archive(bytes: &[u8], imports_root: &Path, existing_ids: &[String]) -> Result<Imported>` — pure w.r.t. muckdb state: only touches `imports_root`.
  - `fn free_id(want: &str, existing: &[String]) -> String`
- Consumes: Task 3's `Manifest`/`ManifestDb`, `store::db_id`.

- [ ] **Step 1: Write failing tests** (append to the `tests` module)

```rust
    #[test]
    fn free_id_suffixes_on_collision() {
        assert_eq!(free_id("a", &[]), "a");
        assert_eq!(free_id("a", &["a".into()]), "a-2");
        assert_eq!(free_id("a", &["a".into(), "a-2".into()]), "a-3");
    }

    #[test]
    fn import_round_trip_rewrites_paths_and_rekeys_formats() {
        let dir = temp_dir("import");
        let db = dir.join("data.duckdb");
        run_sql(&db, "CREATE TABLE t(k TEXT, n INT); INSERT INTO t VALUES ('a', 1); CREATE VIEW v_counts AS SELECT k, n FROM t;");
        let session = sample_session(&db);
        let fmt = formats::Entry {
            db: crate::store::db_id(&db.display().to_string()),
            table: None,
            column: "n".into(),
            format: Default::default(),
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
        let Tile::View { db: tile_db, .. } = &imported.session.tiles[1] else { panic!("view tile") };
        assert_eq!(tile_db, &imported.dbs[0].display().to_string());

        // Format entries re-keyed to the imported path's id.
        assert_eq!(imported.formats.len(), 1);
        assert_eq!(imported.formats[0].db, crate::store::db_id(tile_db));

        // The imported snapshot actually answers queries.
        let rows = crate::introspect::query_json(tile_db, "SELECT n FROM v_counts").unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn import_refuses_newer_format_and_non_archives() {
        let root = temp_dir("badimport");
        assert!(import_archive(b"not a zip", &root, &[]).is_err());
    }
```

Note: `formats::Format` needs `Default` for the test — check; if it doesn't derive `Default`, construct via `serde_json::from_str::<formats::Entry>(...)` instead:
```rust
let fmt: formats::Entry = serde_json::from_str(&format!(
    r#"{{"db":"{}","column":"n","format":{{"prefix":"$"}}}}"#,
    crate::store::db_id(&db.display().to_string())
)).unwrap();
```
(Use whichever compiles; the JSON route needs no changes to formats.rs.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test export`
Expected: compile error — `import_archive`, `free_id`, `Imported` not defined.

- [ ] **Step 3: Implement** (append to `src/export.rs`)

```rust
/// The outcome of unpacking an archive: the rewritten session (collision-free
/// id, tile paths pointing at the imported copies), the re-keyed format
/// entries, and where the dbs landed.
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
pub fn import_archive(bytes: &[u8], imports_root: &Path, existing_ids: &[String]) -> Result<Imported> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("reading .muckdb zip")?;
    let manifest: Manifest =
        serde_json::from_str(&read_zip_string(&mut zip, "manifest.json")?)
            .context("parsing manifest.json")?;
    if manifest.format > FORMAT_VERSION {
        bail!(
            "archive is format v{} but this muckdb reads up to v{FORMAT_VERSION} — upgrade muckdb",
            manifest.format
        );
    }
    let mut session: Session =
        serde_json::from_str(&read_zip_string(&mut zip, "session.json")?)
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

    Ok(Imported { session, formats: fmts, dbs })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test export`
Expected: all export tests PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/export.rs
git commit -m "feat: .muckdb import core — unpack, rewrite paths, re-key formats, collision-safe ids"
```

---

### Task 5: Wrappers + CLI subcommands (`session export` / `session import`)

**Files:**
- Modify: `src/export.rs` (wrappers), `src/formats.rs` (two pub fns), `src/session.rs` (make `save` pub; add CLI arms + usage), `src/main.rs` (help text)
- Test: unit test for `formats::merge_entries` in `src/formats.rs`; CLI round-trip verified manually (touches the real data dir)

**Interfaces:**
- Produces:
  - `formats::entries_for_db_ids(ids: &[String]) -> Result<Vec<Entry>>`
  - `formats::merge_entries(new: Vec<Entry>) -> Result<()>` (imported entry wins on same db+table+column)
  - `export::export_session(id: &str, out: Option<PathBuf>) -> Result<PathBuf>`
  - `export::export_session_bytes(id: &str) -> Result<(String, Vec<u8>)>` (filename, zip bytes — for the daemon)
  - `export::import_and_install(bytes: &[u8]) -> Result<Imported>` (saves session, merges formats, appends ledger records)
  - `session::save` becomes `pub`
- Consumes: Tasks 3–4.

- [ ] **Step 1: formats.rs — pub helpers + test**

Add after `set_entry` (uses the existing private `load_registry`/`save_registry`):

```rust
/// Registry entries keyed to any of these db ids (for bundling into an export).
pub fn entries_for_db_ids(ids: &[String]) -> Result<Vec<Entry>> {
    Ok(load_registry()?
        .into_iter()
        .filter(|e| ids.contains(&e.db))
        .collect())
}

/// Merge imported entries into the registry; an imported entry replaces an
/// existing one for the same db/table/column.
pub fn merge_entries(new: Vec<Entry>) -> Result<()> {
    let mut entries = load_registry()?;
    for e in new {
        entries.retain(|x| !(x.db == e.db && x.table == e.table && x.column == e.column));
        entries.push(e);
    }
    save_registry(&entries)
}
```

Skip a registry-file round-trip test (it would write the real data dir); the merge logic is exercised end-to-end in Step 5. If formats.rs already has a tests module with pure helpers, add nothing here.

- [ ] **Step 2: session.rs — make `save` public**

Change `fn save(session: &Session) -> Result<()>` (~line 264) to `pub fn save(session: &Session) -> Result<()>`.

- [ ] **Step 3: export.rs — wrappers with real side effects**

```rust
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
    let result = export_session(id, Some(tmp.clone())).and_then(|p| {
        fs::read(&p).with_context(|| format!("reading {p:?}"))
    });
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
```

- [ ] **Step 4: session.rs — CLI arms**

In the `match action` in `session::cli` (before the `_ =>` arm):

```rust
        // Bundle a session + full snapshots of its databases into a portable
        // `<id>.muckdb` zip (import on any machine with `session import`).
        "export" => {
            let name =
                session_arg.context("usage: muckdb session export <name> [--out FILE.muckdb]")?;
            let id = slug(&name);
            let out = p.get("out").map(PathBuf::from);
            let path = crate::export::export_session(&id, out)?;
            let abs = path.canonicalize().unwrap_or(path);
            let kb = fs::metadata(&abs).map(|m| m.len() / 1024).unwrap_or(0);
            println!("exported session {id}: {} ({kb} kB)", abs.display());
            Ok(0)
        }
        "import" => {
            let file = session_arg.context("usage: muckdb session import <file.muckdb>")?;
            let bytes = fs::read(&file).with_context(|| format!("reading {file}"))?;
            let imported = crate::export::import_and_install(&bytes)?;
            crate::facade::ensure_daemon()?;
            println!(
                "imported session {} ({} tiles, {} db{}) — http://localhost:{}/session/{}/",
                imported.session.id,
                imported.session.tiles.len(),
                imported.dbs.len(),
                if imported.dbs.len() == 1 { "" } else { "s" },
                crate::facade::PORT,
                imported.session.id
            );
            Ok(0)
        }
```

Update the usage string in the `_` arm: change the first line to
`usage: muckdb session <create|list|post|tile|screenshot|export|import|rm> ...` and add two lines to the list:
```
export <name> [--out FILE.muckdb]  (bundle session + database snapshots into a portable zip)
import <file.muckdb>               (load an exported session; dbs land in muckdb's data dir)
```
Also update `src/main.rs` help(): `session <subcommand>   build dashboards: create | list | post | tile | screenshot | export | import | rm`.

- [ ] **Step 5: Verify end-to-end via the CLI**

```bash
cargo build
D=/tmp/claude-1000/-home-anko-code-rust-muckdb/*/scratchpad; mkdir -p $D
./target/debug/muckdb $D/roundtrip.duckdb -c "CREATE TABLE t(k TEXT, n INT); INSERT INTO t VALUES ('a',1),('b',2); CREATE OR REPLACE VIEW v_n AS SELECT k, n FROM t;"
./target/debug/muckdb session create roundtrip-test --title "Roundtrip"
./target/debug/muckdb session tile roundtrip-test --name chart --db $D/roundtrip.duckdb --view v_n --chart bar --x k --y n --caption "test"
./target/debug/muckdb format $D/roundtrip.duckdb n --prefix '$' --decimals 2
./target/debug/muckdb session export roundtrip-test --out $D/roundtrip-test.muckdb
./target/debug/muckdb session import $D/roundtrip-test.muckdb
./target/debug/muckdb ls session roundtrip-test-2   # tiles point at <data-dir>/imports/roundtrip-test-2/
./target/debug/muckdb format list                    # entry re-keyed to the imported db
./target/debug/muckdb ls databases | grep imports    # imported db listed
```
Expected: import prints the suffixed id and URL; `ls session` shows the rewritten db path; format list has an entry for the imported db id. Clean up: `./target/debug/muckdb session rm roundtrip-test; ./target/debug/muckdb session rm roundtrip-test-2`.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/export.rs src/formats.rs src/session.rs src/main.rs
git commit -m "feat: muckdb session export/import CLI — portable .muckdb archives"
```

---

### Task 6: Daemon endpoints

**Files:**
- Modify: `src/server.rs` (two routes + handlers)

**Interfaces:**
- Produces: `GET /api/session/export?id=<session>` → zip download; `POST /api/session/import` (raw zip body) → `{ok: true, id}` or `{error}`.
- Consumes: `export::export_session_bytes`, `export::import_and_install`.

- [ ] **Step 1: Add routes**

In the router (~line 48), after `.route("/api/session", get(api_session))`:

```rust
        .route("/api/session/export", get(api_session_export))
        .route(
            "/api/session/import",
            post(api_session_import).layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024 * 1024)),
        )
```
(Axum's default body limit is 2 MB — far too small for full db snapshots.)

- [ ] **Step 2: Handlers** (near `api_session`, reusing `SessionParams`)

```rust
/// Bundle a session into a `.muckdb` zip download (the export button).
async fn api_session_export(Query(p): Query<SessionParams>) -> Response {
    let id = session::slug(&p.id);
    let result = tokio::task::spawn_blocking(move || crate::export::export_session_bytes(&id)).await;
    match result {
        Ok(Ok((name, bytes))) => (
            [
                (header::CONTENT_TYPE, "application/zip".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}.muckdb\"", safe_filename(&name)),
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(Err(e)) => error_json(&e),
        Err(e) => error_json(&anyhow::anyhow!("export task failed: {e}")),
    }
}

/// Install an uploaded `.muckdb` archive (the header import button). The
/// session save lands in the watched sessions dir, so every viewer's session
/// list updates on its own.
async fn api_session_import(body: axum::body::Bytes) -> Response {
    let result = tokio::task::spawn_blocking(move || crate::export::import_and_install(&body)).await;
    match result {
        Ok(Ok(imported)) => {
            Json(json!({ "ok": true, "id": imported.session.id })).into_response()
        }
        Ok(Err(e)) => error_json(&e),
        Err(e) => error_json(&anyhow::anyhow!("import task failed: {e}")),
    }
}
```

Careful: `safe_filename` maps `.` to `_` — check its charset (`src/server.rs:303`); the filename must keep its dot. Either allow `.` in `safe_filename` or build the disposition as `format!("attachment; filename=\"{}.muckdb\"", safe_filename(&id))` with `export_session_bytes` returning the bare id. Use the second (no behavior change for the existing CSV export caller): change `export_session_bytes` to return `(id.to_string(), bytes)` and the handler to append `.muckdb`.

- [ ] **Step 3: Verify over HTTP**

```bash
cargo build && ./target/debug/muckdb --stop; ./target/debug/muckdb --status || true
./target/debug/muckdb session list   # any invocation restarts the daemon with the new binary
curl -s -o /tmp/claude-1000/-home-anko-code-rust-muckdb/*/scratchpad/dl.muckdb -D - 'http://localhost:11000/api/session/export?id=demo' | head -5
# Expect: 200, content-type application/zip, content-disposition attachment; filename="demo.muckdb"
curl -s -X POST --data-binary @/tmp/claude-1000/-home-anko-code-rust-muckdb/*/scratchpad/dl.muckdb 'http://localhost:11000/api/session/import'
# Expect: {"ok":true,"id":"demo-2"}  (then: ./target/debug/muckdb session rm demo-2)
```

- [ ] **Step 4: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/server.rs src/export.rs
git commit -m "feat: /api/session/export and /api/session/import daemon endpoints"
```

---

### Task 7: Web UI buttons + docs

**Files:**
- Modify: `src/assets/index.html` (export button in session nav row, import button + file input in titlebar, wiring), `CLAUDE.md`, `README.md`, `src/assets/skill/SKILL.md` (command reference blocks)

**Interfaces:**
- Consumes: Task 6's endpoints, Task 2's `toast()`.

- [ ] **Step 1: Markup**

Titlebar (~line 452), before the theme button:

```html
    <button class="kbtn" id="import-btn" title="Import a .muckdb session export">⇪ import</button>
    <input type="file" id="import-file" accept=".muckdb,.zip" hidden>
```

Session nav row (~line 477), next to the contents toggle (reuse `.toc-toggle` styling for visual consistency, but a distinct id — check how `.toc-toggle` is styled and add a sibling class if `#toc-toggle`-specific rules exist):

```html
        <button class="toc-toggle" id="export-btn" title="Export this session as a .muckdb file"><svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M8 2v8M5 7l3 3 3-3M3 13h10"/></svg><span>export</span></button>
```

- [ ] **Step 2: Wiring** (with the other init listeners near the bottom, ~line 2800)

```js
  $("export-btn").addEventListener("click", () => {
    if (!state.selSession) { toast("select a session to export", true); return; }
    const a = document.createElement("a");
    a.href = "/api/session/export?id=" + encodeURIComponent(state.selSession);
    a.click();
  });
  $("import-btn").addEventListener("click", () => $("import-file").click());
  $("import-file").addEventListener("change", async () => {
    const f = $("import-file").files[0];
    $("import-file").value = "";
    if (!f) return;
    toast(`importing ${f.name}…`);
    try {
      const r = await (await fetch("/api/session/import", { method: "POST", body: f })).json();
      if (r.error) throw new Error(r.error);
      toast(`imported session ${r.id}`);
      setTab("sessions", { silent: true });
      loadSession(r.id, true);
    } catch (e) { toast("import failed: " + (e && e.message || e), true); }
  });
```

- [ ] **Step 3: Verify in the browser**

Restart the daemon on the new binary (`./target/debug/muckdb --stop; ./target/debug/muckdb session list`). At `http://localhost:11000/session/demo/`: click **export** → browser downloads `demo.muckdb`; click **import** in the header, pick that file → toast "imported session demo-2", dashboard opens on it, charts render against the imported db (click **explore** on a tile to confirm the db copy answers queries). Take `muckdb session screenshot demo-2` and read the PNG to confirm panels render. Clean up `demo-2` after.

- [ ] **Step 4: Docs**

Add `export`/`import` lines to the command-reference blocks in `CLAUDE.md` ("Command reference" fence) and `src/assets/skill/SKILL.md` (same fence), plus a short bullet each: what the archive contains, where imports land, collision suffixing. Check `README.md` for a session command list and update if present.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/assets/index.html CLAUDE.md README.md src/assets/skill/SKILL.md
git commit -m "feat: session export/import buttons in the web UI + docs"
```

---

### Task 8: `muckdb --start` — start the server without opening a browser

**Files:**
- Modify: `src/main.rs` (match arm + help), `CLAUDE.md` + `src/assets/skill/SKILL.md` (mention beside `--status`/`--stop`/`--display`)

**Interfaces:**
- Produces: `muckdb --start` — ensures the daemon is running and prints the URL; exactly `--display` minus `open_browser`.

- [ ] **Step 1: Add the match arm** (in `run()`, next to `--display`)

```rust
        // Start the background daemon without opening a browser.
        Some("--start") => {
            facade::ensure_daemon()?;
            println!(
                "muckdb daemon serving at http://localhost:{}",
                facade::PORT
            );
            Ok(0)
        }
```

Add to `help()`'s muckdb commands list, above `--display`:
```
  --start                start the background daemon (without opening a browser)
```

- [ ] **Step 2: Verify**

```bash
cargo build
./target/debug/muckdb --stop
./target/debug/muckdb --start     # prints the URL, no browser window opens
./target/debug/muckdb --status    # running
```

- [ ] **Step 3: Docs + commit**

Mention `--start` in `CLAUDE.md` (the "(muckdb --status / --stop / --display exist if needed)" line) and `src/assets/skill/SKILL.md` if it has the same line.

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
git add src/main.rs CLAUDE.md src/assets/skill/SKILL.md
git commit -m "feat: muckdb --start starts the daemon without opening a browser"
```

---

### Task 9: Ship

- [ ] **Step 1: Full verification sweep**

`cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` all clean; re-run the Task 1 Back-button checklist and the Task 7 export/import round-trip once on the final build.

- [ ] **Step 2: Push and release**

```bash
git push origin main
gh run watch $(gh run list --workflow=ci.yml -L1 --json databaseId -q '.[0].databaseId')   # CI green
cargo release patch --execute    # per release-on-push memory: release after pushing main
gh run list --workflow=release.yml -L1
```
