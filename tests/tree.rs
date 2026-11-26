//! `tree` contracts: structure, glyphs per locale, ignore rules, and depth.

mod common;

use common::*;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

fn run_tree(cwd: &Path, locale: &str) -> String {
    let result = focal(
        cwd,
        &env(&[
            ("PATH", ""),
            ("LC_ALL", locale),
            ("FOCAL_OMIT_WORKSPACE", "1"),
        ]),
        &["tree"],
        None,
    );
    assert!(result.status.success(), "{}", text(&result.stderr));
    text(&result.stdout)
}

fn tree_structure(locale: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("fixture");
    fs::create_dir_all(root.join("d/empty")).unwrap();
    let run = write(&root, "d/run", "");
    fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).unwrap();
    symlink("d", root.join("link")).unwrap();
    write(&root, ".gitignore", "omit\n");
    write(&root, ".ignore", "d\n");
    write(&root, "omit", "");
    fs::create_dir(root.join("build")).unwrap();
    write(&root, "z", "");
    let stdout = run_tree(&root, locale);
    let (branch, end, bar) = if locale == "C" {
        ("|-- ", "`-- ", "|   ")
    } else {
        ("├── ", "└── ", "│\u{a0}\u{a0} ")
    };
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "<context>");
    // tree versions differ in whether -F annotates the root and symlink target.
    assert!(lines[1] == "." || lines[1] == "./", "{stdout}");
    let expected = [
        format!("{branch}.gitignore"),
        format!("{branch}.ignore"),
        format!("{branch}d/"),
        format!("{bar}{branch}empty/"),
        format!("{bar}{end}run*"),
        format!("{branch}link -> d/"),
        format!("{end}z"),
        "</context>".to_owned(),
    ];
    assert_eq!(lines[2..], expected, "{stdout}");
}

#[test]
fn tree_structure_c() {
    tree_structure("C");
}

#[test]
fn tree_structure_utf8() {
    tree_structure("en_US.UTF-8");
}

#[test]
fn tree_depth() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("1/2/3/4/5/6/7")).unwrap();
    let stdout = run_tree(tmp.path(), "C");
    assert!(stdout.contains("6/"), "{stdout}");
    assert!(!stdout.contains("7/"), "{stdout}");
}
