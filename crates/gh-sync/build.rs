#![allow(missing_docs)]

use std::process::Command;

fn main() {
    // Re-run when the git HEAD changes (new commits, branch switches).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");

    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| String::from("unknown"), |s| s.trim().to_owned());

    let rustc_version = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| String::from("unknown"), |s| s.trim().to_owned());

    println!("cargo:rustc-env=GIT_HASH={git_hash}");
    println!("cargo:rustc-env=RUSTC_VERSION_INFO={rustc_version}");

    // Declare `coverage_nightly` as a known cfg key so that rustc's
    // `unexpected_cfg` lint does not fire on stable builds.
    // cargo-llvm-cov sets this flag automatically when running on nightly.
    println!("cargo::rustc-check-cfg=cfg(coverage_nightly)");
}
