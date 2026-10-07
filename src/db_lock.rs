//! Cross-process coordination for database access through muckdb.
//!
//! Keep the guard alive until DuckDB exits. Locks are advisory: direct DuckDB
//! clients and databases attached inside arbitrary SQL do not participate.
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

pub(crate) struct DatabaseLock {
    _file: File,
}

fn database_path(db: &str) -> Result<PathBuf> {
    let path = Path::new(db);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Resolve existing ancestors too, so a new database through a symlinked
    // directory has the same identity before and after it is created.
    fn resolve(path: &Path) -> PathBuf {
        if let Ok(path) = path.canonicalize() {
            return path;
        }
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => resolve(parent).join(name),
            _ => path.to_path_buf(),
        }
    }
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(resolve(&normalized))
}

pub(crate) fn acquire(db: &str, readonly: bool) -> Result<DatabaseLock> {
    use std::hash::{Hash, Hasher};
    let path = database_path(db)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    let dir = crate::paths::data_dir()?.join("database-locks");
    std::fs::create_dir_all(&dir)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(dir.join(format!("{:016x}.lock", hash.finish())))
        .with_context(|| format!("opening coordination lock for {}", path.display()))?;
    lock(&file, readonly)?;
    // Never unlink lock files: waiters must continue using the same inode.
    Ok(DatabaseLock { _file: file })
}

fn lock(file: &File, readonly: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let operation = if readonly {
            libc::LOCK_SH
        } else {
            libc::LOCK_EX
        };
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error).context("acquiring database coordination lock");
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (file, readonly);
        anyhow::bail!("database coordination locks require Unix");
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn readers_share_and_writers_wait_until_all_readers_exit() {
        let path = std::env::temp_dir().join(format!("muckdb-lock-test-{}", std::process::id()));
        let open = || {
            OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .unwrap()
        };
        let first = open();
        let second = open();
        let writer = open();
        lock(&first, true).unwrap();
        lock(&second, true).unwrap();
        assert_ne!(
            unsafe { libc::flock(writer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        drop(first);
        assert_ne!(
            unsafe { libc::flock(writer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        drop(second);
        lock(&writer, false).unwrap();
        let reader = open();
        assert_ne!(
            unsafe { libc::flock(reader.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) },
            0
        );
        drop(writer);
        lock(&reader, true).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn aliases_and_new_files_share_identity() {
        let dir =
            std::env::temp_dir().join(format!("muckdb-lock-path-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("alias")).unwrap();
        let real = dir.join("real/new.duckdb");
        let alias = dir.join("alias/new.duckdb");
        let expected = database_path(real.to_str().unwrap()).unwrap();
        assert_eq!(expected, database_path(alias.to_str().unwrap()).unwrap());
        std::fs::write(&real, []).unwrap();
        assert_eq!(expected, database_path(alias.to_str().unwrap()).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
