use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=GUIX_P2P_BUILD_COMMIT");
    if std::path::Path::new(".git").exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
        println!("cargo:rerun-if-changed=.git/refs");
        println!("cargo:rerun-if-changed=.git/packed-refs");
    }

    let commit = std::env::var("GUIX_P2P_BUILD_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GUIX_P2P_BUILD_COMMIT={commit}");
}

fn git_commit() -> Option<String> {
    let output = Command::new("git").args(["rev-parse", "--short=12", "HEAD"]).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let commit = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!commit.is_empty()).then_some(commit)
}
