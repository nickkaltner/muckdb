//! `muckdb skill ...` — install the bundled Claude Code skill (a guide that
//! teaches coding agents how to drive muckdb) into the user's skills directory.
//!
//! The skill content is embedded in the binary, so this works whether muckdb was
//! built from source or installed via Homebrew (which only drops the binary in
//! `bin` and can't write to `$HOME` itself).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// The skill markdown, baked into the binary at build time.
pub const SKILL_MD: &str = include_str!("assets/skill/SKILL.md");

/// `~/.claude/skills/muckdb/SKILL.md` — where Claude Code looks for user skills.
fn skill_path() -> Result<PathBuf> {
    let home = directories::BaseDirs::new()
        .context("could not locate your home directory")?
        .home_dir()
        .to_path_buf();
    Ok(home.join(".claude/skills/muckdb/SKILL.md"))
}

/// Dispatch `muckdb skill <install|uninstall|path>`.
pub fn cli(args: &[String]) -> Result<i32> {
    match args.first().map(String::as_str) {
        Some("install") => install(args.contains(&"--force".to_string())),
        Some("uninstall" | "remove" | "rm") => uninstall(),
        Some("path") => {
            println!("{}", skill_path()?.display());
            Ok(0)
        }
        _ => {
            eprintln!(
                "usage: muckdb skill <install|uninstall|path>\n\n  \
                 install [--force]   write the muckdb skill to ~/.claude/skills/muckdb/SKILL.md\n  \
                 uninstall           remove the installed muckdb skill\n  \
                 path                print where the skill would be installed"
            );
            Ok(2)
        }
    }
}

/// Write the embedded skill to the user's skills directory.
fn install(force: bool) -> Result<i32> {
    let dest = skill_path()?;
    if dest.exists() && !force {
        println!(
            "muckdb skill already installed at {}\n(re-run with --force to overwrite)",
            dest.display()
        );
        return Ok(0);
    }
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(&dest, SKILL_MD).with_context(|| format!("writing {}", dest.display()))?;
    println!("installed muckdb skill → {}", dest.display());
    println!("Restart Claude Code (or start a new session) to pick it up.");
    Ok(0)
}

/// Remove the installed skill (and its now-empty `muckdb` skill directory).
fn uninstall() -> Result<i32> {
    let dest = skill_path()?;
    if !dest.exists() {
        println!(
            "muckdb skill is not installed (nothing at {})",
            dest.display()
        );
        return Ok(0);
    }
    fs::remove_file(&dest).with_context(|| format!("removing {}", dest.display()))?;
    // Clean up the per-skill directory if we left it empty.
    if let Some(dir) = dest.parent()
        && dir.read_dir().is_ok_and(|mut e| e.next().is_none())
    {
        let _ = fs::remove_dir(dir);
    }
    println!("removed muckdb skill ({})", dest.display());
    Ok(0)
}
