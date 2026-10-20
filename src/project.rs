use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

pub const CAM_DIR: &str = ".cam";
pub const DB_NAME: &str = "cam.db";

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
}

impl Project {
    pub fn resolve(explicit: Option<&Path>) -> Result<Self> {
        if let Some(path) = explicit {
            return Ok(Self {
                root: path
                    .canonicalize()
                    .with_context(|| format!("cannot resolve project path {}", path.display()))?,
            });
        }

        let cwd = std::env::current_dir()?;
        let root = resolve_implicit_root(&cwd)?;
        Ok(Self { root })
    }

    pub fn cam_dir(&self) -> PathBuf {
        self.root.join(CAM_DIR)
    }

    pub fn db_path(&self) -> PathBuf {
        self.cam_dir().join(DB_NAME)
    }

    pub fn init(path: Option<&Path>) -> Result<Self> {
        let project = match path {
            Some(p) => Self::resolve(Some(p))?,
            None => {
                let cwd = std::env::current_dir()?;
                if is_sensitive_root(&cwd) {
                    bail!(
                        "Refusing to automatically initialize home directory or system root as project: '{}'. \
                         Specify path explicitly if you really intend to target this directory.",
                        cwd.display()
                    );
                }
                Self { root: cwd }
            }
        };
        project.ensure_initialized()?;
        Ok(project)
    }

    pub fn ensure_initialized(&self) -> Result<()> {
        self.connect().map(|_| ())
    }

    pub(crate) fn connect(&self) -> Result<Connection> {
        crate::db::open_db(&self.db_path())
    }
}

pub fn resolve_implicit_root(start: &Path) -> Result<PathBuf> {
    let resolved = find_project_root(start);
    if is_sensitive_root(&resolved) {
        bail!(
            "Refusing to automatically use '{}' as project root. \
             Specify --project explicitly if you really intend to target this directory.",
            resolved.display()
        );
    }
    Ok(resolved)
}

pub fn find_project_root(start: &Path) -> PathBuf {
    let start_abs = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(start))
            .unwrap_or_else(|_| start.to_path_buf())
    };

    let home = dirs::home_dir();
    let home_canon = home.as_ref().and_then(|h| h.canonicalize().ok());

    // 1. Find nearest .git without escaping home or root boundary
    if let Some(git_root) = walk_up_boundary(&start_abs, home.as_deref(), home_canon.as_deref(), |p| {
        p.join(".git").exists()
    }) {
        return git_root;
    }

    // 2. Find nearest .cam (excluding global ~/.cam)
    if let Some(cam_root) = walk_up_boundary(&start_abs, home.as_deref(), home_canon.as_deref(), |p| {
        p.join(CAM_DIR).exists()
    }) {
        return cam_root;
    }

    // 3. Fallback to start without escaping workspace
    start_abs
}

fn is_boundary(path: &Path, home: Option<&Path>, home_canon: Option<&Path>) -> bool {
    if path.parent().is_none() {
        return true;
    }
    if let Some(h) = home {
        if path == h {
            return true;
        }
        if let (Some(hc), Ok(pc)) = (home_canon, path.canonicalize()) {
            if pc == hc {
                return true;
            }
        }
    }
    false
}

fn walk_up_boundary<F>(
    start: &Path,
    home: Option<&Path>,
    home_canon: Option<&Path>,
    predicate: F,
) -> Option<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    let mut current = start.to_path_buf();
    loop {
        if is_boundary(&current, home, home_canon) {
            return None;
        }
        if predicate(&current) {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

pub fn is_sensitive_root(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }
    if let Some(home) = dirs::home_dir() {
        if path == home {
            return true;
        }
        if let (Ok(p_canon), Ok(h_canon)) = (path.canonicalize(), home.canonicalize()) {
            if p_canon == h_canon {
                return true;
            }
        }
    }

    #[cfg(unix)]
    {
        const SENSITIVE_SYS_DIRS: &[&str] = &[
            "/System",
            "/Library",
            "/Applications",
            "/usr",
            "/var",
            "/etc",
            "/bin",
            "/sbin",
            "/opt",
            "/tmp",
            "/private",
            "/dev",
            "/proc",
            "/sys",
        ];
        for sys in SENSITIVE_SYS_DIRS {
            let sys_path = Path::new(sys);
            if path == sys_path {
                return true;
            }
            if let (Ok(p_canon), Ok(s_canon)) = (path.canonicalize(), sys_path.canonicalize()) {
                if p_canon == s_canon {
                    return true;
                }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Ok(p_canon) = path.canonicalize() {
            if let Ok(windir) = std::env::var("SystemRoot") {
                if let Ok(w_canon) = Path::new(&windir).canonicalize() {
                    if p_canon == w_canon {
                        return true;
                    }
                }
            }
            if let Ok(pf) = std::env::var("ProgramFiles") {
                if let Ok(pf_canon) = Path::new(&pf).canonicalize() {
                    if p_canon == pf_canon {
                        return true;
                    }
                }
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn nearest_git_is_preferred_over_outer_cam() {
        let tmp = tempdir().unwrap();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::create_dir(outer.join(CAM_DIR)).unwrap();
        std::fs::create_dir(inner.join(".git")).unwrap();

        let resolved = find_project_root(&inner);
        assert_eq!(resolved, inner);
    }

    #[test]
    fn nearest_cam_is_found_when_no_git() {
        let tmp = tempdir().unwrap();
        let outer = tmp.path().join("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::create_dir(outer.join(CAM_DIR)).unwrap();

        let resolved = find_project_root(&inner);
        assert_eq!(resolved, outer);
    }

    #[test]
    fn stops_at_workspace_when_no_markers() {
        let tmp = tempdir().unwrap();
        let project = tmp.path().join("my_workspace");
        std::fs::create_dir_all(&project).unwrap();

        let resolved = find_project_root(&project);
        assert_eq!(resolved, project);
    }

    #[test]
    fn sensitive_root_detects_root_and_home() {
        assert!(is_sensitive_root(Path::new("/")));
        if let Some(home) = dirs::home_dir() {
            assert!(is_sensitive_root(&home));
        }
        #[cfg(unix)]
        {
            assert!(is_sensitive_root(Path::new("/Library")));
            assert!(is_sensitive_root(Path::new("/System")));
        }
        let tmp = tempdir().unwrap();
        assert!(!is_sensitive_root(tmp.path()));
    }

    #[test]
    fn resolve_refuses_sensitive_root_without_explicit() {
        if let Some(home) = dirs::home_dir() {
            let res = resolve_implicit_root(&home);
            assert!(res.is_err());
            let err = res.unwrap_err().to_string();
            assert!(err.contains("Refusing to automatically use"), "{err}");
        }
    }

    #[test]
    fn explicit_project_allows_any_valid_path() {
        let tmp = tempdir().unwrap();
        let proj = Project::resolve(Some(tmp.path())).unwrap();
        assert_eq!(proj.root, tmp.path().canonicalize().unwrap());
    }
}
