use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use fs4::fs_std::FileExt;

use crate::project::Project;

const LOCK_FILE: &str = "incremental.lock";

/// Projects whose incremental lock is held by this process.
///
/// `flock` does not conflict with another handle in the same process, so the
/// set is what stops a second watcher here. Other processes are stopped by
/// the OS lock on `.cam/incremental.lock`.
static HELD: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Exclusive claim on a project's incremental updates.
///
/// Held for the life of `cam watch` or the watcher thread inside `cam mcp`.
/// Dropped when that watcher stops; the OS also releases it if the process dies.
pub struct IncrementalLock {
    file: File,
    root: PathBuf,
}

impl IncrementalLock {
    /// Take the lock if no watcher already owns this project.
    pub fn try_acquire(project: &Project) -> Result<Option<Self>> {
        let root = canonical_root(project)?;
        let mut held = lock_set();
        if held.contains(&root) {
            return Ok(None);
        }
        // Only the directory is needed to host the lock file. The database is
        // opened by the update itself, so a waiting second MCP does not reopen
        // SQLite and rerun the schema on every retry.
        let cam_dir = root.join(crate::project::CAM_DIR);
        fs::create_dir_all(&cam_dir).with_context(|| format!("create {}", cam_dir.display()))?;
        let path = cam_dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open incremental lock {}", path.display()))?;
        match file.try_lock_exclusive() {
            Ok(()) => {
                write_owner(&file)?;
                held.insert(root.clone());
                Ok(Some(Self { file, root }))
            }
            Err(err) if is_contended(&err) => Ok(None),
            Err(err) => Err(err).with_context(|| format!("lock {}", path.display())),
        }
    }

    /// Wait until this process can own incremental updates for `project`.
    ///
    /// Returns `Ok(None)` when `stop` fires before the lock becomes free, so a
    /// watcher that is only waiting can still be shut down cleanly.
    pub fn acquire_blocking(
        project: &Project,
        stop: Option<&Receiver<()>>,
    ) -> Result<Option<Self>> {
        if let Some(lock) = Self::try_acquire(project)? {
            return Ok(Some(lock));
        }
        eprintln!(
            "cam incremental lock held for {}; waiting",
            project.root.display()
        );
        loop {
            if stop_requested(stop) {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(200));
            if let Some(lock) = Self::try_acquire(project)? {
                return Ok(Some(lock));
            }
        }
    }
}

fn stop_requested(stop: Option<&Receiver<()>>) -> bool {
    stop.is_some_and(|rx| !matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)))
}

impl Drop for IncrementalLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        lock_set().remove(&self.root);
    }
}

fn lock_set() -> std::sync::MutexGuard<'static, HashSet<PathBuf>> {
    HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn canonical_root(project: &Project) -> Result<PathBuf> {
    project
        .root
        .canonicalize()
        .with_context(|| format!("canonicalize {}", project.root.display()))
}

fn write_owner(mut file: &File) -> Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    writeln!(file, "pid={}", std::process::id())?;
    file.sync_all()?;
    Ok(())
}

fn is_contended(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::AlreadyExists
    ) || matches!(
        err.raw_os_error(),
        // EAGAIN, EWOULDBLOCK, EBUSY, EDEADLK, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        Some(11) | Some(16) | Some(35) | Some(36) | Some(32) | Some(33)
    )
}

/// Run `update` only when this process can own the child's incremental lock.
///
/// `None` means another MCP (or watcher) already owns that checkout, so the
/// caller must leave its `.cam/cam.db` alone.
pub fn with_incremental_lock<T>(
    project: &Project,
    update: impl FnOnce(&Project) -> Result<T>,
) -> Result<Option<T>> {
    let Some(_guard) = IncrementalLock::try_acquire(project)? else {
        return Ok(None);
    };
    Ok(Some(update(project)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn lock_is_exclusive_until_dropped() {
        let dir = tempdir().unwrap();
        let project = Project::init(Some(dir.path())).unwrap();
        let first = IncrementalLock::try_acquire(&project).unwrap();
        assert!(first.is_some());
        assert!(project.cam_dir().join(super::LOCK_FILE).is_file());
        assert!(IncrementalLock::try_acquire(&project).unwrap().is_none());
        drop(first);
        assert!(IncrementalLock::try_acquire(&project).unwrap().is_some());
    }

    #[test]
    fn lock_does_not_open_the_database() {
        let dir = tempdir().unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
        };
        let lock = IncrementalLock::try_acquire(&project).unwrap();
        assert!(lock.is_some());
        assert!(project.cam_dir().join(super::LOCK_FILE).is_file());
        assert!(
            !project.db_path().exists(),
            "taking the lock must not create or open the graph database"
        );
    }
}
