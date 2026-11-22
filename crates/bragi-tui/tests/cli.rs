//! The non-interactive commands, run as a process against the demo nodes.

use std::process::Command;

fn bragi_tui(home: &std::path::Path, root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bragi-tui"))
        .args(["--demo", "--home"])
        .arg(home)
        .arg("--root")
        .arg(root)
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("bragi-tui runs")
}

#[test]
fn rewrite_prints_a_diff_then_applies_and_undoes() {
    let d = tempfile::tempdir().unwrap();
    let (home, root) = (d.path().join("home"), d.path().join("project"));
    std::fs::create_dir_all(&root).unwrap();
    let original = "fn main() {\n    println!(\"hi\");\n}\n";
    std::fs::write(root.join("main.rs"), original).unwrap();

    let out = bragi_tui(&home, &root, &["--rewrite", "main.rs:2", "greet louder"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("--- a/main.rs\n+++ b/main.rs\n@@ -1,3 +1,4 @@\n"), "{stdout}");
    assert!(stdout.contains(": greet louder\n"), "{stdout}");
    assert_eq!(std::fs::read_to_string(root.join("main.rs")).unwrap(), original, "a diff only, without --apply");

    let out = bragi_tui(&home, &root, &["code", "--rewrite", "main.rs:2", "--apply", "greet louder"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bragi-tui undo"));
    assert!(std::fs::read_to_string(root.join("main.rs")).unwrap().contains("greet louder"));

    let out = Command::new(env!("CARGO_BIN_EXE_bragi-tui")).arg("--home").arg(&home).arg("undo").output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_to_string(root.join("main.rs")).unwrap(), original);
}

#[test]
fn usage_errors_and_version() {
    // A throwaway home, so nothing here could reach a real profile.
    let home = tempfile::tempdir().unwrap();
    let cmd = || {
        let mut c = Command::new(env!("CARGO_BIN_EXE_bragi-tui"));
        c.env("HOME", home.path()).env("XDG_DATA_HOME", home.path()).env_remove("BRAGI_TUI_HOME");
        c
    };
    let out = cmd().arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("bragi-tui {}", env!("CARGO_PKG_VERSION")));
    let out = cmd().args(["code", "--suggest"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "suggest without context: {}", String::from_utf8_lossy(&out.stderr));
    let out = cmd().arg("frob").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
