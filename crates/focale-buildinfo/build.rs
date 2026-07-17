//! Embeds the short git commit hash as `FOCALE_GIT_HASH` at build time.
//!
//! Best-effort by design: any failure (no `git` binary, building from a
//! source tarball or vendored checkout) falls back to `"unknown"` — the
//! hash feeds a debug-only provenance field and must never fail the build.
//! `git commit --amend` or packed refs can leave a stale hash until the
//! next rebuild of this crate; acceptable for the same reason.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn main() {
    let hash = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=FOCALE_GIT_HASH={hash}");

    // Re-run when HEAD moves so the hash tracks new commits.
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={git_dir}/{reference}");
        }
    }
}
