//! `release-context` GitHub enrichment, against a local fake of the REST and GraphQL APIs.
//!
//! The fake answers GraphQL association queries per commit from the request body, serves
//! REST routes in order, and can delay a reply to exercise the per-request timeout and the
//! shared collection deadline. The shared `FakeGitHub` cannot answer GraphQL from the
//! request body or delay replies, so this file runs its own small `tiny_http` server.
//! Nothing here contacts real GitHub.

mod common;

use common::*;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const SLUG: &str = "octo/widget";

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Value,
    headers: Vec<(String, String)>,
    delay: Duration,
}

impl Reply {
    fn new(status: u16, body: Value, headers: &[(&str, &str)]) -> Self {
        Self {
            status,
            body,
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            delay: Duration::ZERO,
        }
    }

    fn ok(body: Value) -> Self {
        Self::new(200, body, &[])
    }
}

/// A request the fake received: method, target (path with query), lower-cased headers.
type Seen = (String, String, HashMap<String, String>);

#[derive(Default)]
struct ApiState {
    routes: HashMap<String, Vec<Reply>>,
    associations: HashMap<String, Vec<Value>>,
    overflow: HashSet<String>,
    graphql: Option<Reply>,
    graphql_delay: HashMap<String, Duration>,
    requests: Vec<Seen>,
}

impl ApiState {
    fn rest(&mut self, target: &str) -> Reply {
        let path = target.split('?').next().unwrap_or(target);
        for key in [target, path] {
            if let Some(replies) = self.routes.get_mut(key)
                && !replies.is_empty()
            {
                return if replies.len() > 1 {
                    replies.remove(0)
                } else {
                    replies[0].clone()
                };
            }
        }
        // Served when a test registers nothing for the path.
        if path == format!("/repos/{SLUG}") {
            Reply::ok(json!({"full_name": SLUG, "html_url": format!("https://github.com/{SLUG}")}))
        } else if path == format!("/repos/{SLUG}/releases") {
            Reply::ok(json!([]))
        } else {
            Reply::new(404, json!({"message": "Not Found"}), &[])
        }
    }

    fn answer_graphql(&self, body: &[u8]) -> Reply {
        if let Some(reply) = &self.graphql {
            return reply.clone();
        }
        let request: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let query = request["query"].as_str().unwrap_or("");
        let aliases = Regex::new(r#"(c\d+): object\(oid: "([0-9a-f]+)"\)"#).unwrap();
        let mut data = Map::new();
        let mut delay = Duration::ZERO;
        for caps in aliases.captures_iter(query) {
            let (alias, sha) = (&caps[1], &caps[2]);
            delay = delay.max(self.graphql_delay.get(sha).copied().unwrap_or_default());
            let nodes: Vec<Value> = self
                .associations
                .get(sha)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .map(|pr| {
                    json!({
                        "number": pr["number"],
                        "title": pr["title"],
                        "body": pr["body"],
                        "url": pr["html_url"],
                        "mergedAt": pr["merged_at"],
                        "mergeCommit": if pr["merge_commit_sha"].is_null() {
                            Value::Null
                        } else {
                            json!({"oid": pr["merge_commit_sha"]})
                        },
                        "author": {
                            "login": pr["user"]["login"],
                            "__typename": pr["user"]["type"],
                        },
                        "labels": {
                            "nodes": pr["labels"],
                            "pageInfo": {"hasNextPage": false},
                        },
                    })
                })
                .collect();
            data.insert(
                alias.to_owned(),
                json!({
                    "associatedPullRequests": {
                        "pageInfo": {"hasNextPage": self.overflow.contains(sha)},
                        "nodes": nodes,
                    }
                }),
            );
        }
        let mut reply = Reply::ok(json!({
            "data": {
                "repository": data,
                "rateLimit": {"remaining": 4000, "resetAt": "2001-01-01T01:00:00Z"},
            }
        }));
        reply.delay = delay;
        reply
    }
}

/// REST routes keyed by path (a full path with query wins), plus a GraphQL endpoint that
/// answers `object(oid: ...)` aliases from `associations`. Requests are served concurrently.
struct FakeApi {
    url: String,
    state: Arc<Mutex<ApiState>>,
    server: Arc<tiny_http::Server>,
    worker: Option<thread::JoinHandle<()>>,
}

impl FakeApi {
    fn start() -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("bind fake"));
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let state = Arc::new(Mutex::new(ApiState::default()));
        let worker = {
            let server = Arc::clone(&server);
            let state = Arc::clone(&state);
            thread::spawn(move || {
                for mut request in server.incoming_requests() {
                    let state = Arc::clone(&state);
                    thread::spawn(move || {
                        let mut body = Vec::new();
                        let _ = request.as_reader().read_to_end(&mut body);
                        let method = request.method().as_str().to_owned();
                        let target = request.url().to_owned();
                        let headers = request
                            .headers()
                            .iter()
                            .map(|h| {
                                (
                                    h.field.as_str().as_str().to_ascii_lowercase(),
                                    h.value.as_str().to_owned(),
                                )
                            })
                            .collect();
                        let reply = {
                            let mut state = state.lock().unwrap();
                            state
                                .requests
                                .push((method.clone(), target.clone(), headers));
                            if method == "POST" {
                                state.answer_graphql(&body)
                            } else {
                                state.rest(&target)
                            }
                        };
                        thread::sleep(reply.delay);
                        let mut response = tiny_http::Response::from_data(
                            serde_json::to_vec(&reply.body).unwrap(),
                        )
                        .with_status_code(reply.status)
                        .with_header(
                            tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .unwrap(),
                        );
                        for (name, value) in reply.headers {
                            response.add_header(
                                tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
                                    .unwrap(),
                            );
                        }
                        // Clients that time out close the socket mid-response.
                        let _ = request.respond(response);
                    });
                }
            })
        };
        Self {
            url,
            state,
            server,
            worker: Some(worker),
        }
    }

    fn add(&self, path: &str, body: Value) {
        self.add_reply(path, Reply::ok(body));
    }

    fn add_reply(&self, path: &str, reply: Reply) {
        self.state
            .lock()
            .unwrap()
            .routes
            .entry(path.to_owned())
            .or_default()
            .push(reply);
    }

    /// A `Link` header pointing at `path` as the next page.
    fn link(&self, path: &str) -> (String, String) {
        (
            "Link".to_owned(),
            format!("<{}{path}>; rel=\"next\"", self.url),
        )
    }

    fn set_associations(&self, associations: HashMap<String, Vec<Value>>) {
        self.state.lock().unwrap().associations = associations;
    }

    fn set_overflow<I: IntoIterator<Item = String>>(&self, shas: I) {
        self.state.lock().unwrap().overflow = shas.into_iter().collect();
    }

    fn set_graphql(&self, reply: Reply) {
        self.state.lock().unwrap().graphql = Some(reply);
    }

    fn set_graphql_delay(&self, delay: HashMap<String, Duration>) {
        self.state.lock().unwrap().graphql_delay = delay;
    }

    fn requests(&self) -> Vec<Seen> {
        self.state.lock().unwrap().requests.clone()
    }

    fn paths(&self, fragment: &str) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|(_, path, _)| path)
            .filter(|path| path.contains(fragment))
            .collect()
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// `2001-01-01T00:00:00+00:00` plus `minutes`, in Python's isoformat.
fn date_after(minutes: u32) -> String {
    let day = 1 + minutes / 1440;
    let hour = (minutes % 1440) / 60;
    let minute = minutes % 60;
    format!("2001-01-{day:02}T{hour:02}:{minute:02}:00+00:00")
}

const ALICE: &str = "Alice <alice@example.com>";

/// A Git repository with an octo/widget origin and fixed, distinct commit dates.
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
            &format!("https://github.com/{SLUG}.git"),
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

struct Fixture {
    _tmp: TempDir,
    repo: Repo,
    api: FakeApi,
    env: Env,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let repo = Repo::new(tmp.path().join("repo"));
    let api = FakeApi::start();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let empty = tmp.path().join("bin");
    fs::create_dir_all(&empty).unwrap();
    let env = env(&[
        // No gh: credentials come from GH_TOKEN alone.
        ("PATH", empty.to_str().unwrap()),
        ("HOME", home.to_str().unwrap()),
        ("LC_ALL", "C"),
        ("NO_COLOR", "1"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GH_TOKEN", "test-token"),
        ("FOCAL_GITHUB_API_URL", &api.url),
    ]);
    Fixture {
        _tmp: tmp,
        repo,
        api,
        env,
    }
}

fn pr_full(
    number: u64,
    author: &str,
    merged: bool,
    merge_commit: Option<&str>,
    bot: bool,
) -> Value {
    json!({
        "number": number,
        "title": format!("Change {number}"),
        "body": format!("Intent for #{number}\n<!-- hidden template -->"),
        "html_url": format!("https://github.com/{SLUG}/pull/{number}"),
        "merged_at": if merged { json!("2001-02-01T00:00:00Z") } else { Value::Null },
        "merge_commit_sha": merge_commit,
        "user": {"login": author, "type": if bot { "Bot" } else { "User" }},
        "labels": [{"name": "enhancement"}],
    })
}

fn pr(number: u64) -> Value {
    pr_full(number, "alice", true, None, false)
}

fn unmerged(number: u64) -> Value {
    pr_full(number, "alice", false, None, false)
}

/// `base` with the fields of `extra` replacing its own.
fn merged_with(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Value::Object(extra)) = (base.as_object_mut(), extra) {
        base.extend(extra);
    }
    base
}

fn release(tag: &str) -> Value {
    json!({"tag_name": tag, "draft": false, "prerelease": false})
}

fn view(tag: &str) -> Value {
    json!({
        "tag_name": tag,
        "name": format!("Title of {tag}"),
        "body": format!("## Highlights of {tag}"),
        "html_url": format!("https://github.com/{SLUG}/releases/tag/{tag}"),
    })
}

fn run(f: &Fixture, env: &Env, args: &[&str]) -> (String, String, Duration) {
    let mut full = vec!["release-context"];
    full.extend_from_slice(args);
    let started = Instant::now();
    let output = focal(&f.repo.root, env, &full, None);
    let elapsed = started.elapsed();
    assert!(output.status.success(), "{}", text(&output.stderr));
    (text(&output.stdout), text(&output.stderr), elapsed)
}

fn section<'a>(out: &'a str, heading: &str) -> &'a str {
    out.split(&format!("\n## {heading}"))
        .nth(1)
        .unwrap_or_else(|| panic!("no section {heading:?} in:\n{out}"))
        .split("\n## ")
        .next()
        .unwrap()
}

fn post_count(api: &FakeApi) -> usize {
    api.requests()
        .iter()
        .filter(|(method, _, _)| method == "POST")
        .count()
}

#[test]
fn enriched_report_groups_commits_and_pins_assets() {
    let mut f = fixture();
    let repo = &mut f.repo;
    repo.commit("Initial", "README.md");
    repo.git(&["tag", "v1.0.0"]);
    let first = repo.commit("Fix one", "one.txt");
    let second = repo.commit("Fix two", "docs/demo.png");
    let direct = repo.commit("Direct push", "direct.txt");
    let bot = repo.commit_as("Bump lib", "deps.txt", "dependabot[bot] <bot@example.com>");
    let head = bot.clone();
    f.api.set_associations(HashMap::from([
        (first.clone(), vec![pr(1)]),
        (second.clone(), vec![pr(1)]),
        (
            bot.clone(),
            vec![merged_with(
                pr_full(3, "dependabot", true, None, true),
                json!({"body": "Verbose bot notes"}),
            )],
        ),
    ]));
    f.api.add(
        &format!("/repos/{SLUG}/releases"),
        json!([release("v1.0.0"), release("v0.9.0")]),
    );
    f.api.add(
        &format!("/repos/{SLUG}/releases/tags/v1.0.0"),
        view("v1.0.0"),
    );
    let (out, _, _) = run(&f, &f.env, &[]);

    let index = section(&out, "PR Index (all associated PRs)");
    assert!(index.contains(&format!(
        "* [#1: Change 1](https://github.com/{SLUG}/pull/1) — 2 in-range commits"
    )));
    assert!(index.contains("#3: Change 3"));
    let prs = section(&out, "Pull Requests");
    assert_eq!(prs.matches("### PR #1: Change 1").count(), 1);
    assert!(prs.contains("**Author:** @alice"));
    assert!(prs.contains("**Labels:** enhancement"));
    assert!(prs.contains("Intent for #1"));
    assert!(!out.contains("hidden template"));
    assert!(prs.contains(&format!("* `{}` — Fix one (Alice)", &first[..12])));
    assert!(prs.contains("* `docs/demo.png`"));
    assert!(prs.contains("* `one.txt`"));
    assert!(!out.contains("Verbose bot notes"));
    assert!(!prs.contains("*All associated PRs"));
    let unmatched = section(&out, "Unmatched Commits");
    assert!(unmatched.contains(&format!("* `{}` — Direct push (Alice)", &direct[..12])));
    assert!(!unmatched.contains(&first[..12]));
    let bots = section(&out, "Bot Updates (compact summary)");
    assert!(bots.contains(&format!(
        "* PR #3: Change 3 — https://github.com/{SLUG}/pull/3; 1 commits, 1 paths"
    )));

    let reference = section(&out, "Previous Release (style reference)");
    assert!(reference.contains("**Tag:** v1.0.0\n**Title:** Title of v1.0.0"));
    assert!(out.contains("## Highlights of v1.0.0"));
    let assets = section(&out, "Repository Assets (pinned to release commit)");
    assert!(assets.contains(&format!(
        "* `docs/demo.png` — [file](https://github.com/{SLUG}/blob/{head}/docs/demo.png), \
         [raw](https://github.com/{SLUG}/raw/{head}/docs/demo.png)"
    )));

    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains(
        "Collected 4 commits; 2 merged PRs; 1 unmatched commits; 0 unresolved association lookups; 0 ambiguous associations."
    ));
    assert!(notes.contains("Bot policy: summary; 1 bot PRs and 1 commits summarized."));
    assert!(notes.contains("1 published release tags were not reachable in the local checkout"));
    assert!(notes.contains("nearest locally reachable published stable release"));
    assert!(notes.contains(
        "GitHub collection made 4 requests; per-request timeout 10s, total GitHub budget 60s."
    ));
    assert!(!notes.contains("WARNING"));
    assert!(f.api.requests().iter().all(|(_, _, headers)| {
        headers.get("authorization").map(String::as_str) == Some("Bearer test-token")
    }));
    assert_eq!(post_count(&f.api), 1);
}

#[test]
fn ambiguous_association_prefers_the_merge_commit() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.repo.git(&["tag", "v1.0.0"]);
    let sha = f.repo.commit("Shared fix", "app.txt");
    f.api.set_associations(HashMap::from([(
        sha.clone(),
        vec![
            pr(1),
            pr_full(2, "alice", true, Some(&sha), false),
            unmerged(3),
        ],
    )]));
    let (out, _, _) = run(&f, &f.env, &[]);
    assert!(out.contains("### PR #2: Change 2"));
    assert!(!out.contains("### PR #1:"));
    assert!(!section(&out, "PR Index (all associated PRs)").contains("#3"));
    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains(&format!(
        "Commit `{}` has multiple merged PR associations: #1, #2; grouped under #2.",
        &sha[..12]
    )));
    assert!(notes.contains(
        "1 merged PRs; 0 unmatched commits; 0 unresolved association lookups; 1 ambiguous associations."
    ));
}

#[test]
fn association_pages_are_followed_and_page_two_failures_stay_unresolved() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.repo.git(&["tag", "v1.0.0"]);
    let complete = f.repo.commit("Popular commit", "a.txt");
    let broken = f.repo.commit("Broken listing", "b.txt");
    f.api.set_overflow([complete.clone(), broken.clone()]);
    for (sha, second) in [(&complete, None), (&broken, Some(422u16))] {
        let base = format!("/repos/{SLUG}/commits/{sha}/pulls?per_page=100");
        let next = format!("{base}&page=2");
        let mut first_page = Reply::ok(json!([
            unmerged(5),
            unmerged(if second.is_some() { 7 } else { 6 })
        ]));
        first_page.headers.push(f.api.link(&next));
        f.api.add_reply(&base, first_page);
        match second {
            Some(status) => f.api.add_reply(
                &next,
                Reply::new(status, json!({"message": "Validation Failed"}), &[]),
            ),
            None => f.api.add(&next, json!([pr(6)])),
        }
    }
    let (out, _, _) = run(&f, &f.env, &[]);
    assert!(out.contains("### PR #6: Change 6"));
    assert!(
        section(&out, "Pull Requests")
            .contains(&format!("* `{}` — Popular commit (Alice)", &complete[..12]))
    );
    assert!(
        section(&out, "Unmatched Commits")
            .contains(&format!("* `{}` — Broken listing (Alice)", &broken[..12]))
    );
    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains("1 merged PRs; 1 unmatched commits; 1 unresolved association lookups"));
    assert!(notes.contains(&format!(
        "WARNING: PR enrichment is incomplete for: `{}`",
        &broken[..12]
    )));
    assert!(notes.contains("Unresolved association lookups: 1 failed, 0 skipped"));
    assert!(notes.contains(&format!(
        "WARNING: GitHub request failed: GET repos/{SLUG}/commits/{broken}/pulls?per_page=100&page=2: HTTP 422: Validation Failed"
    )));
    assert_eq!(f.api.paths("&page=2").len(), 2);
}

#[test]
fn deadline_expiry_midway_keeps_completed_results() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.repo.git(&["tag", "v1.0.0"]);
    let quick = f.repo.commit("Quick", "quick.txt");
    let slow: Vec<String> = (0..10)
        .map(|i| f.repo.commit(&format!("Slow {i}"), &format!("slow{i}.txt")))
        .collect();
    f.api
        .set_associations(HashMap::from([(quick.clone(), vec![pr(1)])]));
    f.api.set_overflow(slow.iter().cloned());
    for (number, sha) in (10..).zip(&slow) {
        let mut reply = Reply::ok(json!([pr(number)]));
        reply.delay = Duration::from_millis(1200);
        f.api.add_reply(
            &format!("/repos/{SLUG}/commits/{sha}/pulls?per_page=100"),
            reply,
        );
    }
    let (out, err, elapsed) = run(
        &f,
        &f.env,
        &["--request-timeout", "5", "--github-timeout", "2"],
    );
    // Four listings finish in the first wave; the next four are capped by the remaining
    // budget and time out; the last two are never requested.
    assert!(elapsed < Duration::from_secs(6), "{elapsed:?}");
    let prs = section(&out, "Pull Requests");
    assert!(prs.contains("### PR #1: Change 1"));
    for number in 10..14 {
        assert!(
            prs.contains(&format!("### PR #{number}:")),
            "#{number}\n{out}"
        );
    }
    for number in 14..20 {
        assert!(
            !prs.contains(&format!("### PR #{number}:")),
            "#{number}\n{out}"
        );
    }
    let notes = section(&out, "Completeness Notes");
    assert!(
        notes.contains("6 unresolved association lookups"),
        "{notes}"
    );
    assert!(
        notes.contains("Unresolved association lookups: 4 failed, 2 skipped"),
        "{notes}"
    );
    assert!(
        Regex::new(
            r"WARNING: GET repos/\S+ timed out after 0\.\ds; remaining GitHub enrichment skipped\."
        )
        .unwrap()
        .is_match(notes),
        "{notes}"
    );
    assert!(notes.contains("total GitHub budget 2s"));
    assert_eq!(f.api.paths("/pulls?per_page=100").len(), 8);
    assert!(err.contains("timed out"), "{err}");
}

#[test]
fn rate_limit_stops_scheduling() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.repo.git(&["tag", "v1.0.0"]);
    let shas: Vec<String> = (0..3)
        .map(|i| f.repo.commit(&format!("Change {i}"), &format!("f{i}.txt")))
        .collect();
    f.api.set_graphql(Reply::new(
        403,
        json!({"message": "API rate limit exceeded for user."}),
        &[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "978310000"),
        ],
    ));
    let (out, _, _) = run(&f, &f.env, &[]);
    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains(
        "WARNING: GitHub rate limit reached; rerun after the limit resets (no automatic retries); \
         remaining GitHub enrichment skipped. Local Git evidence is preserved."
    ));
    assert!(notes.contains("3 unresolved association lookups"));
    assert!(notes.contains("Unresolved association lookups: 0 failed, 3 skipped"));
    assert!(f.api.paths("/pulls").is_empty());
    for sha in &shas {
        assert!(section(&out, "Unmatched Commits").contains(&sha[..12]));
    }
}

#[test]
fn graphql_errors_fall_back_to_rest_listings() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.repo.git(&["tag", "v1.0.0"]);
    let sha = f.repo.commit("Fix", "app.txt");
    f.api.set_graphql(Reply::ok(
        json!({"data": null, "errors": [{"message": "Something went wrong"}]}),
    ));
    f.api.add(
        &format!("/repos/{SLUG}/commits/{sha}/pulls?per_page=100"),
        json!([pr(4)]),
    );
    let (out, _, _) = run(&f, &f.env, &[]);
    assert!(out.contains("### PR #4: Change 4"));
    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains("WARNING: GitHub request failed: POST graphql:"));
    assert!(notes.contains("Something went wrong"));
    assert!(notes.contains("0 unresolved association lookups"));
}

#[test]
fn missing_credentials_keep_the_local_report() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.repo.git(&["tag", "v1.0.0"]);
    f.repo.commit("Change", "app.txt");
    let env: Env = f
        .env
        .iter()
        .filter(|(name, _)| name != "GH_TOKEN")
        .cloned()
        .collect();
    let (out, _, _) = run(&f, &env, &[]);
    let notes = section(&out, "Completeness Notes");
    assert!(notes.contains(
        "WARNING: GitHub repository lookup failed; all commits are shown without PR enrichment."
    ));
    assert!(
        notes.contains("GitHub enrichment unavailable: no GitHub credentials found for github.com")
    );
    assert!(notes.contains("GitHub collection made 0 requests"));
    assert!(notes.contains("1 unresolved association lookups"));
    assert!(f.api.requests().is_empty());
    assert!(section(&out, "Unmatched Commits").contains("Change (Alice)"));
}

#[test]
fn previous_release_is_the_nearest_published_one_before_a_historical_head() {
    let mut f = fixture();
    let repo = &mut f.repo;
    let base = repo.commit("Initial", "README.md");
    repo.git(&["tag", "v1.0.0"]);
    repo.commit("Patch", "patch.txt");
    repo.git(&["tag", "v1.0.1"]);
    let historical = repo.commit("Minor", "minor.txt");
    repo.git(&["tag", "v1.1.0"]);
    repo.commit("Major", "major.txt");
    repo.git(&["tag", "v2.0.0"]);
    let first = format!("/repos/{SLUG}/releases?per_page=100");
    let next = format!("{first}&page=2");
    let mut page = Reply::ok(json!([
        release("v2.0.0"),
        release("v1.1.0"),
        merged_with(release("v1.2.0-rc.1"), json!({"prerelease": true})),
    ]));
    page.headers.push(f.api.link(&next));
    f.api.add_reply(&first, page);
    f.api.add(
        &next,
        json!([
            release("v1.0.1"),
            release("v1.0.0"),
            merged_with(release("draft"), json!({"draft": true})),
        ]),
    );
    f.api.add(
        &format!("/repos/{SLUG}/releases/tags/v1.0.1"),
        view("v1.0.1"),
    );
    let (out, _, _) = run(&f, &f.env, &["--base", &base, "--head", &historical]);
    assert!(out.contains("**Tag:** v1.0.1\n**Title:** Title of v1.0.1"));
    assert!(out.contains("my GitHub release `v1.1.0`"));
    assert!(!section(&out, "Previous Release (style reference)").contains("v2.0.0"));
    assert_eq!(
        f.api.paths("/releases/tags/"),
        [format!("/repos/{SLUG}/releases/tags/v1.0.1")]
    );
}

#[test]
fn explicit_previous_release_and_failed_lookup() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.api.add(
        &format!("/repos/{SLUG}/releases/tags/style%2Fexample"),
        merged_with(view("style/example"), json!({"body": null})),
    );
    let (out, _, _) = run(&f, &f.env, &["--previous-release", "style/example"]);
    assert!(out.contains("**Tag:** style/example"));
    assert!(out.contains("*No description provided.*"));
    assert!(out.contains("explicitly selected with --previous-release: style/example"));
    assert!(f.api.paths("/releases?").is_empty());

    let (out, _, _) = run(&f, &f.env, &["--previous-release", "missing"]);
    assert!(out.contains("*Previous release `missing` unavailable: release lookup failed.*"));
    assert!(
        out.contains("WARNING: could not fetch previous release missing; style reference omitted.")
    );
}

#[test]
fn output_order_is_independent_of_completion_order() {
    let mut f = fixture();
    f.repo.commit("Initial", "README.md");
    f.repo.git(&["tag", "v1.0.0"]);
    let shas: Vec<String> = (0..45)
        .map(|i| f.repo.commit(&format!("Change {i}"), &format!("f{i}.txt")))
        .collect();
    f.api.set_associations(
        shas.iter()
            .enumerate()
            .map(|(i, sha)| (sha.clone(), vec![pr(100 - i as u64 / 5)]))
            .collect(),
    );
    let mut outputs = Vec::new();
    for slow in [&shas[0], &shas[44]] {
        f.api
            .set_graphql_delay(HashMap::from([(slow.clone(), Duration::from_millis(500))]));
        let (out, _, _) = run(&f, &f.env, &["--max-tokens", "0"]);
        outputs.push(out);
    }
    assert_eq!(outputs[0], outputs[1]);
    let headings: Vec<String> = Regex::new(r"(?m)^### PR #(\d+)")
        .unwrap()
        .captures_iter(&outputs[0])
        .map(|c| c[1].to_owned())
        .collect();
    let expected: Vec<String> = (0..9).map(|i| (100 - i).to_string()).collect();
    assert_eq!(headings, expected);
    assert!(outputs[0].contains("45 commits; 9 merged PRs; 0 unmatched commits; 0 unresolved"));
    assert_eq!(post_count(&f.api), 6);
}
