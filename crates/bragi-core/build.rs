//! With the on-device engine on an Apple target, link clang's runtime library for `@available`
//! (`__isPlatformVersionAtLeast`): llama.cpp's Metal backend uses it, and rustc links with
//! `-nodefaultlibs`, so the clang driver doesn't add the library itself. Only a static archive
//! from the toolchain, found with `xcrun clang --print-runtime-dir`; nothing else changes.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var_os("CARGO_FEATURE_ON_DEVICE").is_none() {
        return;
    }
    let var = |k: &str| env::var(k).unwrap_or_default();
    let lib = match var("CARGO_CFG_TARGET_OS").as_str() {
        "macos" => "clang_rt.osx",
        "ios" if var("CARGO_CFG_TARGET_ABI") == "sim" || var("CARGO_CFG_TARGET_ARCH") == "x86_64" => "clang_rt.iossim",
        "ios" => "clang_rt.ios",
        _ => return,
    };
    let dir = Command::new("xcrun")
        .args(["clang", "--print-runtime-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    match dir {
        Some(dir) if dir.join(format!("lib{lib}.a")).is_file() => {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static:-bundle={lib}");
        }
        _ => println!("cargo:warning=lib{lib}.a not found with `xcrun clang --print-runtime-dir`; linking may miss __isPlatformVersionAtLeast"),
    }
}
