use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/browser/linux/helper.c");
    emit_git_hash();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("zeron-webkit");
    let flags = Command::new("pkg-config")
        .args(["--cflags", "--libs", "webkit2gtk-4.1", "json-glib-1.0"])
        .output()
        .expect("pkg-config is required to build the Linux browser helper");
    assert!(
        flags.status.success(),
        "The Linux browser requires WebKitGTK 4.1 and JSON-GLib development files \
         discoverable by pkg-config (webkit2gtk-4.1 and json-glib-1.0). \
         See docs/reference/linux-browser.md for distribution-specific installation commands.\n{}",
        String::from_utf8_lossy(&flags.stderr)
    );
    let status = Command::new(env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args([
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Wno-unused-parameter",
            "src/browser/linux/helper.c",
            "-o",
        ])
        .arg(&output)
        .args(String::from_utf8(flags.stdout).unwrap().split_whitespace())
        .status()
        .expect("C compiler is required to build the Linux browser helper");
    assert!(status.success(), "Linux browser helper compilation failed");
}

fn emit_git_hash() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    // logs/HEAD changes on every commit, reset, and checkout, even when HEAD
    // itself still reads "ref: refs/heads/main".
    if let Some(paths) = git(&["rev-parse", "--git-path", "HEAD", "--git-path", "logs/HEAD"]) {
        for path in paths.lines() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(hash) = git(&["rev-parse", "--short=8", "HEAD"]) {
        println!("cargo:rustc-env=ZERON_GIT_HASH={hash}");
    }
}
