//! `muckdb skill ...` — install the bundled Claude Code skill (a guide that
//! teaches coding agents how to drive muckdb) into the user's skills directory.
//!
//! The skill content is embedded in the binary, so this works whether muckdb was
//! built from source or installed via Homebrew (which only drops the binary in
//! `bin` and can't write to `$HOME` itself).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The skill markdown, baked into the binary at build time.
pub const SKILL_MD: &str = include_str!("assets/skill/SKILL.md");

/// Skill locations supported by the agent runtimes we integrate with. New
/// installs use `.agents`; `--force` migrates legacy `.claude` installs there.
fn skill_paths() -> Result<[PathBuf; 2]> {
    let home = directories::BaseDirs::new()
        .context("could not locate your home directory")?
        .home_dir()
        .to_path_buf();
    Ok(skill_paths_for(&home))
}

fn skill_paths_for(home: &Path) -> [PathBuf; 2] {
    [
        home.join(".claude/skills/muckdb/SKILL.md"),
        home.join(".agents/skills/muckdb/SKILL.md"),
    ]
}

fn installed_paths(paths: &[PathBuf; 2]) -> Vec<PathBuf> {
    paths.iter().filter(|p| p.exists()).cloned().collect()
}

fn clean_empty_skill_dir(dest: &Path) {
    if let Some(dir) = dest.parent()
        && dir.read_dir().is_ok_and(|mut e| e.next().is_none())
    {
        let _ = fs::remove_dir(dir);
    }
}

/// Write the canonical `.agents` copy. When migrating, only remove the legacy
/// path after the new file has been written successfully.
fn write_agent_skill(paths: &[PathBuf; 2], migrate_legacy: bool) -> Result<bool> {
    let dest = &paths[1];
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(dest, SKILL_MD).with_context(|| format!("writing {}", dest.display()))?;
    let migrated = migrate_legacy && paths[0].exists();
    if migrated {
        fs::remove_file(&paths[0]).with_context(|| format!("removing {}", paths[0].display()))?;
        clean_empty_skill_dir(&paths[0]);
    }
    Ok(migrated)
}

/// Dispatch `muckdb skill <install|uninstall|path>`.
pub fn cli(args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("install") => {
            install(args.contains(&"--force".to_string()) || args.contains(&"-f".to_string()))
        }
        Some("uninstall" | "remove" | "rm") => uninstall(),
        Some("path") => {
            let paths = skill_paths()?;
            let existing = installed_paths(&paths);
            for path in if existing.is_empty() {
                vec![paths[1].clone()]
            } else {
                existing
            } {
                println!("{}", path.display());
            }
            Ok(0)
        }
        _ => {
            eprintln!(
                "usage: muckdb skill <install|uninstall|path>\n\n  \
                 install [-f|--force]  install in ~/.agents/skills/muckdb; force migrates ~/.claude there\n  \
                 uninstall           remove every installed muckdb skill copy\n  \
                 path                print the installed skill path(s), or the default install path"
            );
            Ok(2)
        }
    }
}

/// Write the embedded skill to the user's skills directory.
fn install(force: bool) -> Result<i32> {
    let paths = skill_paths()?;
    let existing = installed_paths(&paths);
    if !existing.is_empty() && !force {
        let locations = existing
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        println!(
            "muckdb skill already installed at:\n{locations}\n(re-run with -f or --force to update it in ~/.agents)"
        );
        return Ok(0);
    }
    // New installs and forced upgrades converge on the shared agent-skill
    // location. Write the destination first, then remove the legacy copy: a
    // failed write can never strand the user without their old skill.
    let migrated = write_agent_skill(&paths, force)?;
    if migrated {
        println!("migrated muckdb skill from {}", paths[0].display());
    }
    println!("installed muckdb skill → {}", paths[1].display());
    println!("Start a new agent session to pick it up.");
    Ok(0)
}

/// Remove the installed skill (and its now-empty `muckdb` skill directory).
fn uninstall() -> Result<i32> {
    let paths = skill_paths()?;
    let existing = installed_paths(&paths);
    if existing.is_empty() {
        println!("muckdb skill is not installed in ~/.claude or ~/.agents (nothing to remove)");
        return Ok(0);
    }
    for dest in existing {
        fs::remove_file(&dest).with_context(|| format!("removing {}", dest.display()))?;
        clean_empty_skill_dir(&dest);
        println!("removed muckdb skill ({})", dest.display());
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_paths_cover_both_supported_agent_roots() {
        let paths = skill_paths_for(Path::new("/home/agent"));
        assert_eq!(
            paths[0],
            PathBuf::from("/home/agent/.claude/skills/muckdb/SKILL.md")
        );
        assert_eq!(
            paths[1],
            PathBuf::from("/home/agent/.agents/skills/muckdb/SKILL.md")
        );
    }

    #[test]
    fn force_migrates_legacy_and_converges_dual_installs_on_agents() {
        let root = std::env::temp_dir().join(format!("muckdb-skill-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = skill_paths_for(&root);

        // A normal new install lands in `.agents`.
        assert!(!write_agent_skill(&paths, false).unwrap());
        assert!(paths[1].exists());
        assert!(!paths[0].exists());

        // A legacy-only install moves to `.agents` with force.
        fs::remove_file(&paths[1]).unwrap();
        fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
        fs::write(&paths[0], "legacy").unwrap();
        assert!(write_agent_skill(&paths, true).unwrap());
        assert!(!paths[0].exists());
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), SKILL_MD);

        // Dual copies converge on one current `.agents` installation.
        fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
        fs::write(&paths[0], "stale legacy").unwrap();
        fs::write(&paths[1], "stale agent copy").unwrap();
        assert!(write_agent_skill(&paths, true).unwrap());
        assert!(!paths[0].exists());
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), SKILL_MD);

        let _ = fs::remove_dir_all(root);
    }
}
