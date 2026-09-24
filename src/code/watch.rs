use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecursiveMode, Watcher};

use super::index::{is_source_path, path_is_skipped, sync_project, SyncReport};
use crate::project::Project;

pub const DEFAULT_DEBOUNCE_MS: u64 = 2000;
const MIN_DEBOUNCE_MS: u64 = 100;
const MAX_DEBOUNCE_MS: u64 = 60_000;

pub fn clamp_debounce_ms(ms: u64) -> u64 {
    ms.clamp(MIN_DEBOUNCE_MS, MAX_DEBOUNCE_MS)
}

pub fn watch_project(
    project: &Project,
    debounce: Duration,
    stop: Option<&Receiver<()>>,
    mut on_sync: impl FnMut(&SyncReport),
) -> Result<()> {
    project.ensure_initialized()?;
    on_sync(&sync_project(project)?);

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .context("start filesystem watcher")?;
    watcher
        .watch(&project.root, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", project.root.display()))?;

    let mut pending = false;
    let mut last_event = Instant::now();

    loop {
        if pending && last_event.elapsed() >= debounce {
            on_sync(&sync_project(project)?);
            pending = false;
            continue;
        }

        if stop_requested(stop) {
            return Ok(());
        }

        let timeout = if pending {
            debounce.saturating_sub(last_event.elapsed())
        } else if stop.is_some() {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(3600)
        };

        match rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                if event_should_sync(&project.root, &event) {
                    pending = true;
                    last_event = Instant::now();
                }
            }
            Ok(Err(err)) => {
                eprintln!("warn: watch event: {err}");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn stop_requested(stop: Option<&Receiver<()>>) -> bool {
    stop.is_some_and(|rx| !matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)))
}

fn event_should_sync(root: &Path, event: &Event) -> bool {
    if !kind_is_relevant(event.kind) {
        return false;
    }
    event.paths.iter().any(|path| path_should_sync(root, path))
}

fn kind_is_relevant(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Modify(ModifyKind::Any)
    )
}

fn path_should_sync(root: &Path, path: &Path) -> bool {
    if path_is_skipped(root, path) {
        return false;
    }
    if !path.exists() {
        return true;
    }
    path.is_file() && is_source_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, EventAttributes};

    #[test]
    fn ignores_cam_db_and_build_dirs() {
        let root = Path::new("/tmp/proj");
        assert!(!path_should_sync(root, &root.join(".cam/cam.db")));
        assert!(!path_should_sync(root, &root.join("target/lib.rs")));
        assert!(path_should_sync(root, &root.join("src/missing.rs")));
    }

    #[test]
    fn ignores_camignore_matches_and_linked_checkouts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".camignore"), "gen/\n*.generated.ts\n").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();

        assert!(!path_should_sync(root, &root.join("gen/a.rs")));
        assert!(!path_should_sync(root, &root.join("src/api.generated.ts")));
        assert!(!path_should_sync(root, &root.join("sub/lib.rs")));
        assert!(path_should_sync(root, &root.join("src/lib.rs")));
    }

    #[test]
    fn reacts_to_source_create() {
        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec!["/tmp/proj/src/lib.rs".into()],
            attrs: EventAttributes::new(),
        };
        assert!(kind_is_relevant(event.kind));
        let meta = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec!["/tmp/proj/src/lib.rs".into()],
            attrs: EventAttributes::new(),
        };
        assert!(kind_is_relevant(meta.kind));
    }
}
