//! `release-context` contracts for the local report.
//!
//! GitHub is unreachable here: the binary gets a token and an API URL on a closed local
//! port (and a `gh` stub that reports a refused connection), so it builds the report from
//! local Git alone. These tests pin release membership, commit order, paths, pinned asset
//! revisions, bot policy, focused diffs, project background, and output bounds. Assertions
//! check content and structure rather than Git's byte-exact rendering.

mod common;

use common::*;
use regex::Regex;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// A port nothing listens on.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// `2001-01-01T00:00:00+00:00` plus `minutes`, in Python's isoformat.
fn date_after(minutes: u32) -> String {
    let day = 1 + minutes / 1440;
    let hour = (minutes % 1440) / 60;
    let minute = minutes % 60;
    format!("2001-01-{day:02}T{hour:02}:{minute:02}:00+00:00")
}

const ALICE: &str = "Alice <alice@example.com>";

/// A Git repository whose commits get distinct, fixed dates.
struct Repo {
    root: PathBuf,
    tick: u32,
}

impl Repo {
    fn new(root: PathBuf) -> Self {
        fs::create_dir_all(&root).unwrap();
        let mut repo = Self { root, tick: 0 };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/octo/widget.git",
        ]);
        repo
    }

    fn git(&mut self, args: &[&str]) -> String {
        self.git_as(args, ALICE)
    }

    fn git_as(&mut self, args: &[&str], author: &str) -> String {
        self.tick += 1;
        let when = date_after(self.tick);
        let (name, email) = author.strip_suffix('>').unwrap().split_once(" <").unwrap();
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", "Alice")
            .env("GIT_COMMITTER_EMAIL", "alice@example.com")
            .env("GIT_AUTHOR_DATE", &when)
            .env("GIT_COMMITTER_DATE", &when)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            text(&output.stderr)
        );
        text(&output.stdout).trim().to_owned()
    }

    fn commit(&mut self, subject: &str, path: &str) -> String {
        self.commit_as(subject, path, ALICE)
    }

    fn commit_as(&mut self, subject: &str, path: &str, author: &str) -> String {
        let target = self.root.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let mut contents = fs::read(&target).unwrap_or_default();
        contents.extend_from_slice(format!("{subject}\n").as_bytes());
        fs::write(&target, contents).unwrap();
        self.git(&["add", "-A"]);
        self.git_as(&["commit", "-qm", subject], author);
        self.git(&["rev-parse", "HEAD"])
    }
}

/// The temporary directory, a repository at `tmp/repo`, and the offline environment.
struct Fixture {
    _tmp: TempDir,
    repo: Repo,
    env: Env,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let repo = Repo::new(tmp.path().join("repo"));
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let stub = tmp.path().join("stub");
    fs::create_dir_all(&stub).unwrap();
    let gh = stub.join("gh");
    fs::write(
        &gh,
        "#!/bin/sh\necho 'error connecting to api.github.com: connection refused' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let api = format!("http://127.0.0.1:{}", closed_port());
    let env = env(&[
        ("PATH", stub.to_str().unwrap()),
        ("HOME", home.to_str().unwrap()),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("LC_ALL", "C"),
        ("NO_COLOR", "1"),
        ("GH_TOKEN", "test-token"),
        ("FOCAL_GITHUB_API_URL", &api),
    ]);
    Fixture {
        _tmp: tmp,
        repo,
        env,
    }
}

/// Shas for the `tagged` history.
struct Tagged {
    v1_0_0: String,
    v1_1_0: String,
    v1_1_1: String,
    later: String,
}

impl Tagged {
    fn get(&self, label: &str) -> &str {
        match label {
            "v1.0.0" => &self.v1_0_0,
            "v1.1.0" => &self.v1_1_0,
            "v1.1.1" => &self.v1_1_1,
            "later" => &self.later,
            _ => panic!("no tag {label}"),
        }
    }
}

/// v1.0.0, v1.1.0, and v1.1.1 on main, plus a prerelease, a non-release tag, and a commit
/// after the last release.
fn tagged(repo: &mut Repo) -> Tagged {
    let v1_0_0 = repo.commit("Initial", "README.md");
    repo.git(&["tag", "v1.0.0"]);
    repo.commit("Feature", "feature.txt");
    let v1_1_0 = repo.commit("Minor release", "minor.txt");
    repo.git(&["tag", "v1.1.0"]);
    let v1_1_1 = repo.commit("Patch release", "patch.txt");
    repo.git(&["tag", "v1.1.1"]);
    repo.commit("Prerelease", "rc.txt");
    repo.git(&["tag", "v2.0.0-rc.1"]);
    repo.git(&["tag", "not-a-release"]);
    let later = repo.commit("Later work", "later.txt");
    Tagged {
        v1_0_0,
        v1_1_0,
        v1_1_1,
        later,
    }
}

fn run(cwd: &Path, env: &Env, args: &[&str]) -> (i32, String, String) {
    let mut full = vec!["release-context"];
    full.extend_from_slice(args);
    let output = focal(cwd, env, &full, None);
    (
        output.status.code().unwrap_or(-1),
        text(&output.stdout),
        text(&output.stderr),
    )
}

fn report(cwd: &Path, env: &Env, args: &[&str]) -> String {
    let (code, out, err) = run(cwd, env, args);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.starts_with("<context>\n# Release Notes Request\n"),
        "{out}"
    );
    assert!(out.ends_with("\n</context>\n"), "{out}");
    out
}

/// The body of `## heading`, up to the next second-level heading.
fn section<'a>(out: &'a str, heading: &str) -> &'a str {
    out.split(&format!("\n## {heading}"))
        .nth(1)
        .unwrap_or_else(|| panic!("no section {heading:?} in:\n{out}"))
        .split("\n## ")
        .next()
        .unwrap()
}

/// Whether every fenced block is closed, honoring longer nested fences.
fn balanced(text: &str) -> bool {
    let fence = Regex::new(r"^(`{3,}|~{3,})(.*)$").unwrap();
    let mut open: Option<String> = None;
    for line in text.lines() {
        let Some(caps) = fence.captures(line) else {
            continue;
        };
        let marker = &caps[1];
        let rest = &caps[2];
        match &open {
            None => open = Some(marker.to_owned()),
            Some(current) => {
                if marker.as_bytes()[0] == current.as_bytes()[0]
                    && marker.len() >= current.len()
                    && rest.trim().is_empty()
                {
                    open = None;
                }
            }
        }
    }
    open.is_none()
}

fn strip_context(out: &str) -> &str {
    let out = out.strip_prefix("<context>\n").unwrap_or(out);
    out.strip_suffix("\n</context>\n").unwrap_or(out)
}

fn headings(text: &str, pattern: &str) -> Vec<String> {
    Regex::new(pattern)
        .unwrap()
        .find_iter(text)
        .map(|m| m.as_str().to_owned())
        .collect()
}

// test_level_forms_select_reachable_stable_tags

fn level_forms_select_reachable_stable_tags(args: &[&str], label: &str) {
    let mut f = fixture();
    let shas = tagged(&mut f.repo);
    let out = report(&f.repo.root, &f.env, args);
    assert!(
        out.contains(&format!("# Release Context: {label} to HEAD\n")),
        "{args:?}: {out}"
    );
    assert!(
        out.contains(&format!(
            "**Base:** {label} (`{}`, exclusive)",
            shas.get(label)
        )),
        "{args:?}: {out}"
    );
}

#[test]
fn level_forms_default() {
    level_forms_select_reachable_stable_tags(&[], "v1.1.1");
}

#[test]
fn level_forms_patch() {
    level_forms_select_reachable_stable_tags(&["patch"], "v1.1.1");
}

#[test]
fn level_forms_patch_flag() {
    level_forms_select_reachable_stable_tags(&["--patch"], "v1.1.1");
}

#[test]
fn level_forms_minor() {
    level_forms_select_reachable_stable_tags(&["minor"], "v1.1.0");
}

#[test]
fn level_forms_minor_flag() {
    level_forms_select_reachable_stable_tags(&["--minor"], "v1.1.0");
}

#[test]
fn level_forms_minor_short_flag() {
    level_forms_select_reachable_stable_tags(&["-m"], "v1.1.0");
}

#[test]
fn level_forms_since_minor() {
    level_forms_select_reachable_stable_tags(&["--since", "minor"], "v1.1.0");
}

#[test]
fn level_forms_level_major() {
    level_forms_select_reachable_stable_tags(&["--level", "major"], "v1.0.0");
}

#[test]
fn level_forms_major() {
    level_forms_select_reachable_stable_tags(&["major"], "v1.0.0");
}

#[test]
fn level_forms_major_flag() {
    level_forms_select_reachable_stable_tags(&["--major"], "v1.0.0");
}

// The last level wins, whatever its form.

#[test]
fn level_forms_last_wins_major_then_patch_flag() {
    level_forms_select_reachable_stable_tags(&["major", "--patch"], "v1.1.1");
}

#[test]
fn level_forms_last_wins_minor_flag_then_major() {
    level_forms_select_reachable_stable_tags(&["--minor", "major"], "v1.0.0");
}

#[test]
fn level_forms_last_wins_level_major_then_short_minor() {
    level_forms_select_reachable_stable_tags(&["--level", "major", "-m"], "v1.1.0");
}

#[test]
fn historical_release_from_a_newer_checkout() {
    let mut f = fixture();
    let shas = tagged(&mut f.repo);
    let root = f.repo.root.clone();
    let out = report(&root, &f.env, &["--version", "v1.1.1"]);
    assert!(out.contains("# Release Context: v1.1.0 to v1.1.1\n"));
    assert!(out.contains(&format!("**Head:** `{}` (inclusive)", shas.v1_1_1)));
    assert!(out.contains("my GitHub release `v1.1.1`"));
    assert!(out.contains("**Commits in range:** 1\n"));
    assert!(out.contains("Patch release"));
    assert!(!out.contains("Later work"));
    assert!(!out.contains("later.txt"));
    assert!(!out.contains("rc.txt"));

    // An explicit head overrides the version tag.
    let out = report(&root, &f.env, &["--version", "v1.1.1", "--head", "HEAD"]);
    assert!(out.contains("# Release Context: v1.1.1 to HEAD\n"));
    assert!(out.contains("Later work"));
    assert!(!section(&out, "Unmatched Commits").contains("Patch release"));

    // Explicit historical endpoints, with the version inferred from the head tag.
    let out = report(&root, &f.env, &["--base", "v1.0.0", "--head", "v1.1.0"]);
    assert!(out.contains("# Release Context: v1.0.0 to v1.1.0\n"));
    assert!(out.contains("my GitHub release `v1.1.0`"));
    assert!(out.contains("**Commits in range:** 2\n"));
    assert!(!out.contains("Patch release"));
}

#[test]
fn upcoming_version_drafts_from_head_and_untagged_heads_get_a_placeholder() {
    let mut f = fixture();
    let shas = tagged(&mut f.repo);
    let out = report(&f.repo.root, &f.env, &["--version", "v1.2.0"]);
    assert!(out.contains("# Release Context: v1.1.1 to HEAD\n"));
    assert!(out.contains(&format!("**Head:** `{}`", shas.later)));
    assert!(out.contains("my GitHub release `v1.2.0`"));
    let out = report(&f.repo.root, &f.env, &[]);
    assert!(out.contains("my GitHub release `vX.Y.Z`"));
    assert!(out.contains("do not invent a version"));
}

#[test]
fn root_release_includes_root_commits_and_pins_assets() {
    let mut f = fixture();
    let root = f.repo.commit("Root", "docs/demo space#1.png");
    f.repo.commit("Later", "later.gif");
    let out = report(&f.repo.root, &f.env, &["--head", &root]);
    assert!(out.contains(&format!(
        "# Release Context: Repository beginning to {root}\n"
    )));
    assert!(out.contains(
        "**Base:** Repository beginning (all reachable history, including root commits)"
    ));
    assert!(out.contains("**Commits in range:** 1\n"));
    let unmatched = section(&out, "Unmatched Commits");
    assert!(unmatched.contains(&format!("* `{}` — Root (Alice)", &root[..12])));
    assert!(unmatched.contains("  * `docs/demo space#1.png`"));
    let assets = section(&out, "Repository Assets (pinned to release commit)");
    assert!(assets.contains(&format!("Tracked image/video candidates at `{root}`")));
    assert!(assets.contains(&format!(
        "`<ASSET_URL: docs/demo space#1.png at {root}>` (repository URL unavailable)"
    )));
    assert!(!out.contains("later.gif"));
    assert!(section(&out, "Release Impact").contains("docs/demo space#1.png"));
}

#[test]
fn merge_range_keeps_topological_order_and_first_parent_paths() {
    let mut f = fixture();
    let repo = &mut f.repo;
    repo.commit("Initial", "README.md");
    repo.git(&["tag", "v1.0.0"]);
    repo.commit("Direct one", "one.txt");
    repo.git(&["checkout", "-qb", "feature"]);
    repo.commit("Feature a", "fa.txt");
    repo.commit("Feature b", "fb.txt");
    repo.git(&["checkout", "-q", "main"]);
    repo.commit("Direct two", "two.txt");
    repo.git(&["merge", "-q", "--no-ff", "feature", "-m", "Merge feature"]);
    let expected = repo.git(&[
        "log",
        "--reverse",
        "--topo-order",
        "--format=%H",
        "v1.0.0..HEAD",
    ]);
    let expected: Vec<&str> = expected.split_whitespace().map(|s| &s[..12]).collect();
    let out = report(&f.repo.root, &f.env, &["--max-tokens", "0"]);
    let unmatched = section(&out, "Unmatched Commits");
    let listed: Vec<&str> = Regex::new(r"(?m)^\* `([0-9a-f]{12})`")
        .unwrap()
        .captures_iter(unmatched)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    assert_eq!(listed, expected);
    let merge = unmatched.split("Merge feature (Alice)\n").nth(1).unwrap();
    let lines: Vec<&str> = merge.lines().take(2).collect();
    assert_eq!(lines, ["  * `fa.txt`", "  * `fb.txt`"]);
    assert!(out.contains("**Commits in range:** 5\n"));
}

#[test]
fn divergent_base_reports_set_difference() {
    let mut f = fixture();
    let repo = &mut f.repo;
    repo.commit("Initial", "README.md");
    repo.git(&["checkout", "-qb", "other"]);
    let base = repo.commit("Other", "other.txt");
    repo.git(&["checkout", "-q", "main"]);
    repo.commit("Head", "head.txt");
    let out = report(&f.repo.root, &f.env, &["--base", &base]);
    assert!(out.contains("Base is not an ancestor of head: this is Git set difference"));
    assert!(!section(&out, "Unmatched Commits").contains("Other"));
    assert!(out.contains("**Commits in range:** 1\n"));
}

#[test]
fn shallow_history_warning() {
    let mut f = fixture();
    let root = f.repo.commit("Initial", "README.md");
    fs::write(f.repo.root.join(".git/shallow"), format!("{root}\n")).unwrap();
    let out = report(&f.repo.root, &f.env, &[]);
    assert!(out.contains(
        "WARNING: shallow checkout; available Git history and tag selection may be incomplete"
    ));
}

#[test]
fn offline_output_keeps_local_evidence() {
    let mut f = fixture();
    tagged(&mut f.repo);
    let out = report(&f.repo.root, &f.env, &[]);
    assert!(out.contains("*Previous release unavailable: GitHub repository lookup failed.*"));
    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains(
        "WARNING: GitHub repository lookup failed; all commits are shown without PR enrichment."
    ));
    assert!(notes.contains("WARNING: previous release style reference could not be fetched."));
    assert!(
        notes
            .contains("GitHub connection or service failure; remaining GitHub enrichment skipped.")
    );
    assert!(out.contains("*No merged PR associations found.*"));
    assert!(section(&out, "Unmatched Commits").contains("Later work (Alice)"));
    assert!(section(&out, "Release Impact").contains("later.txt"));
    assert!(notes.contains("Collected 2 commits; 0 merged PRs; 2 unmatched commits;"));
    assert_eq!(
        headings(&out, r"(?m)^#+ .*$"),
        [
            "# Release Notes Request",
            "# Release Context: v1.1.1 to HEAD",
            "## Previous Release (style reference)",
            "## Repository Assets (pinned to release commit)",
            "## PR Index (all associated PRs)",
            "## Pull Requests",
            "## Unmatched Commits",
            "## Release Impact",
            "## Completeness Notes",
        ]
    );
}

#[test]
fn offline_lookups_count_as_unresolved() {
    let mut f = fixture();
    let shas = tagged(&mut f.repo);
    let out = report(&f.repo.root, &f.env, &[]);
    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains("2 unresolved association lookups"));
    let incomplete = notes
        .split("WARNING: PR enrichment is incomplete for: ")
        .nth(1)
        .unwrap()
        .split('\n')
        .next()
        .unwrap();
    assert!(incomplete.contains(&format!("`{}`", &shas.later[..12])));
    assert!(notes.contains("0 failed, 2 skipped"));
}

// test_bot_policy_for_unmatched_commits

fn bot_policy_for_unmatched_commits(mode: &str) {
    let mut f = fixture();
    let repo = &mut f.repo;
    repo.commit("Initial", "README.md");
    repo.git(&["tag", "v1.0.0"]);
    repo.commit("Human change", "app.txt");
    let bot = repo.commit_as(
        "Bump library",
        "deps.txt",
        "dependabot[bot] <bot@example.com>",
    );
    let out = report(&f.repo.root, &f.env, &["--bots", mode]);
    let unmatched = section(&out, "Unmatched Commits");
    assert_eq!(unmatched.contains(&bot[..12]), mode == "include");
    assert_eq!(
        out.contains("## Bot Updates (compact summary)"),
        mode == "summary"
    );
    if mode == "summary" {
        assert!(
            section(&out, "Bot Updates (compact summary)").contains(&format!(
                "* `{}` — Bump library (dependabot[bot])",
                &bot[..12]
            ))
        );
    }
    assert!(section(&out, "Release Impact").contains("deps.txt"));
    let expected = match mode {
        "summary" => "Bot policy: summary; 0 bot PRs and 1 commits summarized.",
        "include" => "Bot policy: include; 0 bot PRs and 1 commits included in full.",
        "exclude" => {
            "Bot policy: exclude; 0 bot PRs and 1 commits omitted from detail (still included in release impact)."
        }
        _ => unreachable!(),
    };
    assert!(out.contains(expected), "{out}");
}

#[test]
fn bot_policy_for_unmatched_commits_summary() {
    bot_policy_for_unmatched_commits("summary");
}

#[test]
fn bot_policy_for_unmatched_commits_include() {
    bot_policy_for_unmatched_commits("include");
}

#[test]
fn bot_policy_for_unmatched_commits_exclude() {
    bot_policy_for_unmatched_commits("exclude");
}

#[test]
fn focused_diff_uses_endpoints_and_repeatable_paths() {
    let mut f = fixture();
    let repo = &mut f.repo;
    let base = repo.commit("Initial", "README.md");
    repo.commit("Wanted", "wanted.txt");
    repo.commit("Nested", "dir/nested.txt");
    repo.commit("Other", "other.txt");
    let historical = repo.git(&["rev-parse", "HEAD"]);
    repo.commit("Not shipped", "wanted.txt");
    let root = f.repo.root.clone();
    let out = report(
        &root,
        &f.env,
        &[
            "--base",
            &base,
            "--head",
            &historical,
            "--diff",
            "--path",
            "wanted.txt",
            "--path",
            "dir/",
        ],
    );
    let focused = section(&out, "Focused Diff");
    assert!(focused.contains("Paths: wanted.txt, dir/"));
    assert!(focused.contains("+Wanted"));
    assert!(focused.contains("+Nested"));
    assert!(!focused.contains("other.txt"));
    assert!(!out.contains("Not shipped"));
    assert!(section(&out, "Release Impact").contains("other.txt"));
    assert!(balanced(&out));

    // Pathspecs resolve from the invocation directory, as with git diff.
    let out = report(
        &root.join("dir"),
        &f.env,
        &["--base", &base, "--diff", "--path", "nested.txt"],
    );
    let focused = section(&out, "Focused Diff");
    assert!(focused.contains("+Nested"));
    assert!(!focused.contains("wanted.txt"));

    let out = report(
        &root,
        &f.env,
        &["--base", &base, "--diff", "--path", "missing.txt"],
    );
    assert!(section(&out, "Focused Diff").contains("No matching changes."));
    let out = report(
        &root,
        &f.env,
        &["--base", &historical, "--head", &historical],
    );
    assert!(section(&out, "Release Impact").contains("No net file changes."));
    assert!(out.contains("**Commits in range:** 0\n"));
}

#[test]
fn project_context_appends_the_current_checkout() {
    let mut f = fixture();
    tagged(&mut f.repo);
    let root = f.repo.root.clone();
    let readme: String = (1..=600).map(|i| format!("line {i}\n")).collect();
    fs::write(root.join("README.md"), readme).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::write(root.join("untracked.txt"), "new\n").unwrap();
    let out = report(&root, &f.env, &["--project-context", "--max-tokens", "0"]);
    let background = out
        .split("\n# Project Background (current checkout, not the release snapshot)\n\n")
        .nth(1)
        .unwrap();
    assert!(background.starts_with(
        "Tree is limited to six levels; core files are limited to their first 500 lines.\n\n# Repository Tree\n```text\n"
    ));
    assert!(
        out.split("# Project Background")
            .next()
            .unwrap()
            .contains("## Completeness Notes")
    );
    assert_eq!(
        headings(background, r"(?m)^# .*$"),
        [
            "# Repository Tree",
            "# Git Info",
            "# README.md",
            "# Cargo.toml",
        ]
    );
    assert!(background.contains("?? untracked.txt"));
    assert!(background.contains("line 500\n```"));
    assert!(!background.contains("line 501"));
    assert!(background.contains("```toml\n[package]\nname = \"x\"\n```"));
    assert!(balanced(&out));
}

#[test]
fn project_context_shares_the_output_budget() {
    let mut f = fixture();
    tagged(&mut f.repo);
    let root = f.repo.root.clone();
    fs::write(
        root.join("README.md"),
        format!("{}\n{}\n", "x".repeat(200), "word ".repeat(20000)),
    )
    .unwrap();
    let out = report(
        &root,
        &f.env,
        &["--project-context", "--max-tokens", "1000"],
    );
    let body = strip_context(&out);
    assert!(body.chars().count() <= 4000, "{}", body.chars().count());
    assert!(body.contains("## Output Limit"));
    assert!(body.contains("output exceeded the approximate 1,000-token budget"));
    assert!(balanced(body));
    assert!(body.contains("## Completeness Notes"));
}

#[test]
fn output_budget_keeps_impact_and_counts() {
    let mut f = fixture();
    let repo = &mut f.repo;
    repo.commit("Initial", "README.md");
    repo.git(&["tag", "v1.0.0"]);
    for index in 0..12 {
        repo.commit(
            &format!("Change number {index} {}", "detail ".repeat(60)),
            &format!("src/file{index:02}.txt"),
        );
    }
    let root = f.repo.root.clone();
    let out = report(&root, &f.env, &["--max-tokens", "2000", "--diff"]);
    let body = strip_context(&out);
    assert!(body.chars().count() <= 8000, "{}", body.chars().count());
    assert!(body.contains("## Output Limit"));
    assert!(
        Regex::new(r"\* [\d,]+ characters of release detail omitted")
            .unwrap()
            .is_match(body)
    );
    // The aggregate impact and final counts move after the omission notice.
    let tail = body.split("## Output Limit").nth(1).unwrap();
    assert!(tail.contains("## Release Impact"));
    assert!(tail.contains("src/file11.txt"));
    assert!(tail.contains("Collected 12 commits; 0 merged PRs; 12 unmatched commits"));
    assert!(balanced(body));

    let full = report(&root, &f.env, &["--max-tokens", "0", "--diff"]);
    assert!(!full.contains("## Output Limit"));
    assert!(!full.contains("## PR Index"));
    assert_eq!(full.matches("Change number").count(), 24);
    assert!(section(&full, "Focused Diff").contains("+Change number 11"));
}

#[test]
fn compact_detail_marks_omitted_paths() {
    let mut f = fixture();
    let repo = &mut f.repo;
    repo.commit("Initial", "README.md");
    repo.git(&["tag", "v1.0.0"]);
    for index in 0..7 {
        fs::write(repo.root.join(format!("many{index}.txt")), "x\n").unwrap();
    }
    repo.commit("Wide change", "wide.txt");
    let root = f.repo.root.clone();
    let out = report(&root, &f.env, &[]);
    assert!(
        section(&out, "Unmatched Commits")
            .contains("  * 3 additional changed paths omitted from detail.")
    );
    let full = report(&root, &f.env, &["--max-tokens", "0"]);
    assert!(!full.contains("additional changed paths omitted"));
    assert!(full.contains("  * `many6.txt`"));
}

// test_argument_errors

fn argument_errors(args: &[&str], message: &str) {
    let mut f = fixture();
    f.repo.commit("Initial", "app.txt");
    let (code, out, err) = run(&f.repo.root, &f.env, args);
    assert_eq!(code, 1, "{args:?}: {err}");
    assert_eq!(out, "");
    assert!(err.contains(message), "{args:?}: {err}");
}

#[test]
fn argument_errors_unknown_argument() {
    argument_errors(&["bogus"], "unknown argument: bogus");
}

#[test]
fn argument_errors_invalid_level() {
    argument_errors(&["--level", "invalid"], "invalid release level 'invalid'");
}

#[test]
fn argument_errors_invalid_bots() {
    argument_errors(&["--bots", "invalid"], "Invalid value for '--bots'");
}

#[test]
fn argument_errors_max_tokens_too_small() {
    argument_errors(
        &["--max-tokens", "500"],
        "--max-tokens must be 0 or at least 1000",
    );
}

#[test]
fn argument_errors_path_without_diff() {
    argument_errors(&["--path", "app.txt"], "--path requires --diff");
}

#[test]
fn help_lists_options() {
    let f = fixture();
    let (code, out, _) = run(&f.repo.root, &f.env, &["--help"]);
    assert_eq!(code, 0);
    for option in [
        "--minor",
        "-m",
        "--major",
        "--patch",
        "--base",
        "--head",
        "--version",
        "--previous-release",
        "--bots",
        "--request-timeout",
        "--github-timeout",
        "--max-tokens",
        "--diff",
        "--path",
        "--project-context",
    ] {
        assert!(out.contains(option), "{option} missing from:\n{out}");
    }
}

// test_usage_errors_come_from_clap

fn usage_errors_come_from_clap(args: &[&str], message: &str) {
    let mut f = fixture();
    f.repo.commit("Initial", "app.txt");
    let (code, _, err) = run(&f.repo.root, &f.env, args);
    assert_eq!(code, 2, "{args:?}: {err}");
    assert!(err.contains(message), "{args:?}: {err}");
}

#[test]
fn usage_errors_come_from_clap_unknown_flag() {
    usage_errors_come_from_clap(&["--unknown-flag"], "unexpected argument '--unknown-flag'");
}

#[test]
fn usage_errors_come_from_clap_missing_value() {
    usage_errors_come_from_clap(&["--base"], "a value is required for '--base <REF>'");
}

#[test]
fn unknown_revision() {
    let mut f = fixture();
    f.repo.commit("Initial", "app.txt");
    let (code, out, err) = run(&f.repo.root, &f.env, &["--head", "does-not-exist"]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(
        err.contains("unknown revision or not a commit: 'does-not-exist'"),
        "{err}"
    );
}
