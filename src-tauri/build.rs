use std::path::Path;
use std::process::Command;

// Compiles the Swift iOS screen-capture helper into the sidecar path Tauri bundles
// (binaries/ios-screen-capture-<target triple>, see bundle.externalBin).
fn build_ios_capture_helper() {
    let source = "helpers/ios-screen-capture.swift";
    println!("cargo:rerun-if-changed={}", source);
    let target = std::env::var("TARGET").expect("TARGET is set by cargo");
    let swift_target = if target.starts_with("aarch64") { "arm64-apple-macos12" } else { "x86_64-apple-macos12" };
    std::fs::create_dir_all("binaries").expect("create binaries dir");
    let out = format!("binaries/ios-screen-capture-{}", target);
    if Path::new(&out).exists() && is_up_to_date(source, &out) {
        return;
    }
    let status = Command::new("xcrun")
        .args(["swiftc", "-O", "-target", swift_target, source, "-o", &out])
        .status()
        .expect("run swiftc");
    assert!(status.success(), "swiftc failed to build {}", source);
}

fn is_up_to_date(source: &str, out: &str) -> bool {
    let modified = |p: &str| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    matches!((modified(source), modified(out)), (Some(src), Some(bin)) if bin >= src)
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_ios_capture_helper();
    }
    tauri_build::build()
}
