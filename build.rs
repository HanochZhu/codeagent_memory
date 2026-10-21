use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    watch_git_head(&manifest_dir);
    println!("cargo:rustc-env=CAM_GIT_HASH={}", git_head(&manifest_dir));
}

fn git_head(dir: &str) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .current_dir(dir)
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let text = text.trim();
            if text.is_empty() {
                "unknown".to_string()
            } else {
                text.to_string()
            }
        }
        _ => "unknown".to_string(),
    }
}

fn watch_git_head(manifest: &str) {
    let git = Path::new(manifest).join(".git");
    if git.is_dir() {
        println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
        return;
    }
    if !git.is_file() {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&git) else {
        return;
    };
    let Some(rest) = text.trim().strip_prefix("gitdir:") else {
        return;
    };
    let gitdir = Path::new(manifest).join(rest.trim());
    println!("cargo:rerun-if-changed={}", gitdir.join("HEAD").display());
}
