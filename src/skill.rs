//! `muckdb skill ...` — install the bundled agent skill (a guide that
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

/// Skill locations supported by the agent runtimes we integrate with. The
/// `.agents` copy is canonical; Claude reads it through a symlink at `.claude`.
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

/// Write the canonical `.agents` copy and link Claude's skill location to it.
fn write_agent_skill(paths: &[PathBuf; 2]) -> Result<()> {
    let claude_dest = &paths[0];
    let agent_dest = &paths[1];
    if let Some(dir) = agent_dest.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    fs::write(agent_dest, SKILL_MD).with_context(|| format!("writing {}", agent_dest.display()))?;
    link_claude_skill(claude_dest, agent_dest)
}

/// Point Claude at the canonical agent skill, replacing an older standalone
/// Claude copy after the canonical copy has been written successfully.
fn link_claude_skill(claude_dest: &Path, agent_dest: &Path) -> Result<()> {
    if let Some(dir) = claude_dest.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if claude_dest.exists() || claude_dest.is_symlink() {
        fs::remove_file(claude_dest)
            .with_context(|| format!("removing {}", claude_dest.display()))?;
    }
    std::os::unix::fs::symlink(agent_dest, claude_dest).with_context(|| {
        format!(
            "linking {} to {}",
            claude_dest.display(),
            agent_dest.display()
        )
    })?;
    Ok(())
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
                 install [-f|--force]  install in ~/.agents and link it from ~/.claude\n  \
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
        // Versions that predate the Claude link may already have the canonical
        // `.agents` copy. Add the missing link without rewriting that copy.
        if paths[1].exists() && !paths[0].exists() && !paths[0].is_symlink() {
            link_claude_skill(&paths[0], &paths[1])?;
            println!("linked Claude skill → {}", paths[0].display());
            println!("Start a new agent session to pick it up.");
            return Ok(0);
        }
        let locations = existing
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        println!(
            "muckdb skill already installed at:\n{locations}\n(re-run with -f or --force to update it)"
        );
        return Ok(0);
    }
    // Write the canonical copy before replacing Claude's path, so a failed
    // write can never strand the user without their existing skill.
    write_agent_skill(&paths)?;
    println!("installed muckdb skill → {}", paths[1].display());
    println!("linked Claude skill → {}", paths[0].display());
    println!("Start a new agent session to pick it up.");
    Ok(0)
}

/// Remove the installed skill (and its now-empty `muckdb` skill directory).
fn uninstall() -> Result<i32> {
    let paths = skill_paths()?;
    let existing = installed_paths(&paths);
    if existing.is_empty() {
        println!("muckdb skill is not installed in ~/.agents or ~/.claude (nothing to remove)");
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
    fn install_writes_agents_copy_and_links_claude_to_it() {
        let root = std::env::temp_dir().join(format!("muckdb-skill-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = skill_paths_for(&root);

        write_agent_skill(&paths).unwrap();
        assert!(paths[1].exists());
        assert!(paths[0].is_symlink());
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), SKILL_MD);
        assert_eq!(fs::read_to_string(&paths[0]).unwrap(), SKILL_MD);

        // An update replaces a legacy standalone Claude copy with the link.
        fs::remove_file(&paths[0]).unwrap();
        fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
        fs::write(&paths[0], "stale legacy").unwrap();
        fs::write(&paths[1], "stale agent copy").unwrap();
        write_agent_skill(&paths).unwrap();
        assert!(paths[0].is_symlink());
        assert_eq!(fs::read_to_string(&paths[1]).unwrap(), SKILL_MD);
        assert_eq!(fs::read_to_string(&paths[0]).unwrap(), SKILL_MD);

        let _ = fs::remove_dir_all(root);
    }
}
