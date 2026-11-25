//! `search`, `symbol`, and `api` contracts.

mod common;

use common::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

fn base_env(tmp: &Path) -> Env {
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();
    env(&[
        ("PATH", ""),
        ("HOME", home.to_str().unwrap()),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("LC_ALL", "C"),
        ("NO_COLOR", "1"),
        ("FOCAL_OMIT_WORKSPACE", "1"),
    ])
}

fn run(cwd: &Path, env: &Env, args: &[&str]) -> Output {
    focal(cwd, env, args, None)
}

/// A Git repository at `tmp/corpus` with matches, Python definitions, hidden, ignored,
/// and binary files.
fn corpus(tmp: &Path) -> PathBuf {
    let root = tmp.join("corpus");
    fs::create_dir_all(root.join("pkg")).unwrap();
    fs::create_dir_all(root.join(".hidden")).unwrap();
    git(&root, &["init", "-q"]);
    let lines: String = (1..=30)
        .map(|n| {
            if [3, 10, 25].contains(&n) {
                "hit\n".to_owned()
            } else {
                format!("l{n}\n")
            }
        })
        .collect();
    write(&root, "lines.txt", lines);
    write(
        &root,
        "pkg/mod.py",
        "class Widget:\n\
         \x20   def method(self):\n\
         \x20       return helper()\n\
         async def _private():\n\
         \x20   pass\n\
         def helper(x=-1):\n\
         \x20   return x\n\
         # class NotAtColumnZero\n",
    );
    write(&root, "pkg/stub.pyi", "def typed() -> int: ...\n");
    write(&root, "pkg/notes.md", "def looks_like_python():\n");
    write(&root, "calls.js", "helper();\n");
    write(&root, ".hidden/secret.py", "def hidden():\n    hit\n");
    write(&root, ".gitignore", "ignored.txt\n");
    write(&root, "ignored.txt", "hit\n");
    write(&root, "binary.dat", b"hit\0data");
    write(&root, "tail.txt", b"no newline hit");
    root
}

/// Groups ripgrep-style output by file, since file order is unspecified.
fn records(payload: &[u8]) -> BTreeMap<String, Vec<String>> {
    let payload = text(payload);
    let body = payload.strip_prefix("<context>\n").unwrap_or(&payload);
    let body = body.strip_suffix("\n</context>\n").unwrap_or(body);
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in body.split('\n') {
        if line == "--" {
            continue;
        }
        let colon = line.find(':');
        let dash = line.get(2..).and_then(|rest| rest.find('-')).map(|i| i + 2);
        let end = [colon, dash]
            .into_iter()
            .flatten()
            .filter(|&i| i > 0)
            .min()
            .unwrap_or_else(|| panic!("no path separator in {line:?}"));
        groups
            .entry(line[..end].to_owned())
            .or_default()
            .push(line.to_owned());
    }
    groups
}

#[test]
fn search_context() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let result = run(&corpus, &env, &["search", "hit"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    let found = records(&result.stdout);
    assert_eq!(
        found.keys().collect::<Vec<_>>(),
        ["./lines.txt", "./tail.txt"]
    );
    let mut lines: Vec<String> = (1..=30).map(|n| format!("./lines.txt-l{n}")).collect();
    for n in [3, 10, 25] {
        lines[n - 1] = "./lines.txt:hit".to_owned();
    }
    // Matches 3 and 10 share one window; 25 starts a new group after a separator.
    let expected: Vec<String> = lines[..15].iter().chain(&lines[19..]).cloned().collect();
    assert_eq!(found["./lines.txt"], expected);
    assert!(
        payload.contains("./lines.txt-l15\n--\n./lines.txt-l20\n"),
        "{payload}"
    );
    assert_eq!(found["./tail.txt"], ["./tail.txt:no newline hit"]);
    assert_eq!(payload.matches("--\n").count(), 2, "{payload}");
    assert!(
        result
            .stderr
            .starts_with(b"done: Search results copied to clipboard (~"),
        "{}",
        text(&result.stderr)
    );
}

fn search_leading_dash(args: &[&str]) {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let mut full = vec!["search"];
    full.extend_from_slice(args);
    let result = run(&corpus, &env, &full);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let found = records(&result.stdout);
    assert!(
        found["./pkg/mod.py"].contains(&"./pkg/mod.py:def helper(x=-1):".to_owned()),
        "{found:?}"
    );
}

#[test]
fn search_leading_dash_after_separator() {
    search_leading_dash(&["--", "-1"]);
}

#[test]
fn search_leading_dash_bare() {
    search_leading_dash(&["-1"]);
}

#[test]
fn search_no_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let result = run(&corpus, &env, &["search", "absent_term"]);
    assert!(result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(
        text(&result.stderr),
        "info: No matches found for 'absent_term'.\n"
    );
}

#[test]
fn search_missing_pattern() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let result = run(&corpus, &env, &["search"]);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        text(&result.stderr),
        "error: missing search pattern\n  hint: usage: focal search <pattern>\n"
    );
}

#[test]
fn search_invalid_regex() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let result = run(&corpus, &env, &["search", "("]);
    assert_eq!(result.status.code(), Some(1));
    assert!(
        text(&result.stderr).contains("error: invalid regex '('"),
        "{}",
        text(&result.stderr)
    );
}

#[test]
fn symbol_includes_call_sites() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let result = run(&corpus, &env, &["symbol", "helper"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let payload = text(&result.stdout);
    let body = payload.strip_prefix("<context>\n# ").unwrap_or(&payload);
    let mut files: Vec<&str> = body
        .split("\n\n# ")
        .map(|block| block.split('\n').next().unwrap())
        .collect();
    files.sort_unstable();
    assert_eq!(files, ["./calls.js", "./pkg/mod.py"], "{payload}");
    assert!(
        payload.contains("# ./calls.js\n```js\nhelper();\n```"),
        "{payload}"
    );
    assert!(payload.ends_with("```\n</context>\n"), "{payload}");
    assert!(
        result
            .stderr
            .starts_with(b"done: Symbol context for 'helper' copied to clipboard (~"),
        "{}",
        text(&result.stderr)
    );
}

#[test]
fn symbol_no_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let result = run(&corpus, &env, &["symbol", "absent_term"]);
    assert!(result.status.success());
    assert_eq!(
        text(&result.stderr),
        "info: No files found containing symbol 'absent_term'.\n"
    );
}

#[test]
fn api_overview() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let corpus = corpus(tmp.path());
    let result = run(&corpus, &env, &["api"]);
    assert!(result.status.success(), "{}", text(&result.stderr));
    let expected: BTreeMap<String, Vec<String>> = [
        (
            "./pkg/mod.py",
            vec![
                "./pkg/mod.py:class Widget:",
                "./pkg/mod.py:async def _private():",
                "./pkg/mod.py:def helper(x=-1):",
            ],
        ),
        (
            "./pkg/stub.pyi",
            vec!["./pkg/stub.pyi:def typed() -> int: ..."],
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.into_iter().map(str::to_owned).collect()))
    .collect();
    assert_eq!(records(&result.stdout), expected);
    assert!(
        result
            .stderr
            .starts_with(b"done: API overview copied to clipboard (~"),
        "{}",
        text(&result.stderr)
    );
}

#[test]
fn api_without_python() {
    let tmp = tempfile::tempdir().unwrap();
    let env = base_env(tmp.path());
    let result = run(tmp.path(), &env, &["api"]);
    assert!(result.status.success());
    assert_eq!(
        text(&result.stderr),
        "info: No Python classes or functions found.\n"
    );
}
