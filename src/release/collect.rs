//! Bounded GitHub collection: one shared deadline, a failure circuit, bounded request
//! concurrency, and per-job diagnostics that keep the output order deterministic.

use crate::github::Client;
use crate::github::track_c::CallError;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::limit::general;
use super::notes::Pr;

/// Requests in flight at once.
pub const CONCURRENCY: usize = 4;
/// Commits per GraphQL association query.
pub const BATCH_SIZE: usize = 20;
/// Longest single wait; keeps `Duration` arithmetic finite for huge timeouts.
const MAX_SECONDS: f64 = 1e9;

/// The result of one request.
pub enum Fetch {
    Ok(Value),
    /// The request was made and failed.
    Failed,
    /// The request was not made because collection had stopped.
    Skipped,
}

/// Shares the deadline and failure circuit across every release GitHub request.
pub struct GitHub {
    client: Option<Client>,
    request_timeout: f64,
    budget: f64,
    deadline: Instant,
    stopped: AtomicBool,
    stop_note: Mutex<Option<String>>,
    requests: AtomicUsize,
}

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value.clamp(0.0, MAX_SECONDS))
}

const RATE_TERMS: [&str; 5] = [
    "rate limit",
    "rate_limit",
    "ratelimited",
    "abuse detection",
    "secondary rate",
];
const AUTH_TERMS: [&str; 5] = [
    "authentication",
    "authenticate",
    "bad credentials",
    "resource not accessible",
    "forbidden",
];

impl GitHub {
    pub fn new(client: Option<Client>, request_timeout: f64, budget: f64) -> Self {
        Self {
            client,
            request_timeout,
            budget,
            deadline: Instant::now() + seconds(budget),
            stopped: AtomicBool::new(false),
            stop_note: Mutex::new(None),
            requests: AtomicUsize::new(0),
        }
    }

    pub fn slug(&self) -> Option<String> {
        self.client.as_ref().map(|client| client.repo.slug())
    }

    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// Opens the circuit: later requests are skipped and the reason is reported once.
    pub fn stop(&self, reason: &str) {
        if !self.stopped.swap(true, Ordering::SeqCst) {
            let note = format!(
                "WARNING: {reason}; remaining GitHub enrichment skipped. Local Git evidence is preserved."
            );
            eprintln!("warning: {note}");
            *self.stop_note.lock().unwrap() = Some(note);
        }
    }

    /// The stop reason, when collection stopped early.
    pub fn stop_note(&self) -> Option<String> {
        self.stop_note.lock().unwrap().clone()
    }

    /// Runs one GET (or a POST with `body`) capped by the remaining budget, recording
    /// non-terminal failures in `notes`.
    pub fn request(&self, path: &str, body: Option<&Value>, notes: &mut Vec<String>) -> Fetch {
        self.exchange(path, body, notes).0
    }

    /// [`Self::request`], plus the next-page link of a successful response.
    fn exchange(
        &self,
        path: &str,
        body: Option<&Value>,
        notes: &mut Vec<String>,
    ) -> (Fetch, Option<String>) {
        let Some(client) = &self.client else {
            return (Fetch::Skipped, None);
        };
        if self.stopped() {
            return (Fetch::Skipped, None);
        }
        let remaining = self
            .deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f64();
        if remaining <= 0.0 {
            self.stop(&format!(
                "GitHub collection exceeded its {}s budget",
                general(self.budget)
            ));
            return (Fetch::Skipped, None);
        }
        let timeout = self.request_timeout.min(remaining);
        let number = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
        let operation = operation(path, body.is_some());
        eprintln!("info: GitHub request {number}: {operation} (timeout {timeout:.1}s)");
        let payload = body.map(Value::to_string);
        let reply = match client.call(path, payload.as_deref(), seconds(timeout)) {
            Ok(reply) => reply,
            Err(CallError::Timeout) => {
                self.stop(&format!("{operation} timed out after {timeout:.1}s"));
                return (Fetch::Failed, None);
            }
            Err(CallError::Transport(message)) => {
                let message = format!("GitHub request failed: {operation}: {message}");
                eprintln!("warning: {message}");
                notes.push(format!("WARNING: {message}"));
                self.stop("GitHub connection or service failure");
                return (Fetch::Failed, None);
            }
        };
        let parsed: Option<Value> = serde_json::from_str(&reply.body).ok();
        let errors = parsed
            .as_ref()
            .and_then(|value| value.get("errors"))
            .and_then(Value::as_array)
            .filter(|errors| !errors.is_empty());
        let error_text = errors
            .map(|errors| Value::from(errors.clone()).to_string())
            .unwrap_or_default()
            .to_lowercase();
        let detail = reply.detail().to_lowercase();
        let has_data = parsed
            .as_ref()
            .and_then(|value| value.get("data"))
            .is_some_and(|data| !data.is_null());
        let rate_limited = reply.status == 429
            || (reply.status == 403
                && (reply.limited || RATE_TERMS.iter().any(|term| detail.contains(term))))
            || RATE_TERMS.iter().any(|term| error_text.contains(term));
        if rate_limited {
            self.stop(
                "GitHub rate limit reached; rerun after the limit resets (no automatic retries)",
            );
            return match parsed {
                Some(value) if has_data => (Fetch::Ok(value), None),
                _ => (Fetch::Failed, None),
            };
        }
        if matches!(reply.status, 401 | 403)
            || AUTH_TERMS.iter().any(|term| error_text.contains(term))
        {
            self.stop("GitHub authentication or permission failure");
            return (Fetch::Failed, None);
        }
        if !reply.ok() || errors.is_some() {
            let reason = match errors {
                Some(errors) if reply.ok() => Value::from(errors.clone()).to_string(),
                _ => format!("HTTP {}: {}", reply.status, reply.detail()),
            };
            let message = format!("GitHub request failed: {operation}: {reason}");
            eprintln!("warning: {message}");
            notes.push(format!("WARNING: {message}"));
            // GraphQL can return useful objects alongside errors for other aliases.
            if let Some(value) = parsed.filter(|_| has_data) {
                return (Fetch::Ok(value), None);
            }
            if reply.status >= 500 {
                self.stop("GitHub connection or service failure");
            }
            return (Fetch::Failed, None);
        }
        let Some(value) = parsed else {
            self.stop(&format!("{operation} returned invalid JSON"));
            return (Fetch::Failed, None);
        };
        if let Some(limit) = value.pointer("/data/rateLimit")
            && limit.get("remaining").and_then(Value::as_i64) == Some(0)
        {
            let reset = limit
                .get("resetAt")
                .and_then(Value::as_str)
                .unwrap_or("the server reset time");
            self.stop(&format!("GitHub GraphQL quota exhausted; reset at {reset}"));
        }
        (Fetch::Ok(value), reply.next)
    }

    /// GETs a JSON array and every following `Link` page, to exhaustion. The error is
    /// [`Fetch::Skipped`] when collection stopped before a page, else [`Fetch::Failed`].
    pub fn get_pages(&self, path: &str, notes: &mut Vec<String>) -> Result<Vec<Value>, Fetch> {
        let mut items = Vec::new();
        let mut next = Some(path.to_owned());
        while let Some(url) = next.take() {
            let (fetch, link) = self.exchange(&url, None, notes);
            let page = match fetch {
                Fetch::Ok(value) => value,
                other => return Err(other),
            };
            let Value::Array(values) = page else {
                let message = format!(
                    "GitHub request failed: {}: expected a JSON array",
                    operation(&url, false)
                );
                eprintln!("warning: {message}");
                notes.push(format!("WARNING: {message}"));
                return Err(Fetch::Failed);
            };
            items.extend(values);
            next = link;
        }
        Ok(items)
    }
}

/// `GET repos/o/r/...` or `POST graphql`, with absolute URLs shortened to their path.
fn operation(path: &str, post: bool) -> String {
    let path = match path.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map_or("", |(_, path)| path),
        None => path.trim_start_matches('/'),
    };
    format!("{} {path}", if post { "POST" } else { "GET" })
}

/// Runs `jobs` on at most `workers` threads and returns their results in job order,
/// whatever order they complete in.
pub fn run_jobs<T: Send, R: Send>(
    workers: usize,
    jobs: Vec<T>,
    work: impl Fn(T) -> R + Sync,
) -> Vec<R> {
    let total = jobs.len();
    let queue = Mutex::new(jobs.into_iter().enumerate());
    let results = Mutex::new((0..total).map(|_| None).collect::<Vec<Option<R>>>());
    thread::scope(|scope| {
        for _ in 0..workers.clamp(1, total.max(1)) {
            scope.spawn(|| {
                loop {
                    let Some((index, job)) = queue.lock().unwrap().next() else {
                        break;
                    };
                    let result = work(job);
                    results.lock().unwrap()[index] = Some(result);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|result| result.expect("every job ran"))
        .collect()
}

/// The GraphQL query for one batch of commits, as the legacy collector builds it.
pub fn association_query(owner: &str, name: &str, shas: &[String]) -> Value {
    let fields: Vec<String> = shas
        .iter()
        .enumerate()
        .map(|(index, sha)| {
            format!(
                "c{index}: object(oid: {}) {{ ... on Commit {{
                associatedPullRequests(first: 100) {{
                    pageInfo {{ hasNextPage }}
                    nodes {{ number title body url mergedAt mergeCommit {{ oid }}
                        author {{ login __typename }}
                        labels(first: 100) {{ nodes {{ name }} pageInfo {{ hasNextPage }} }}
                    }}
                }}
            }} }}",
                Value::from(sha.as_str())
            )
        })
        .collect();
    let query = format!(
        "query {{ repository(owner: {}, name: {}) {{ {} }} rateLimit {{ remaining resetAt }} }}",
        Value::from(owner),
        Value::from(name),
        fields.join(" ")
    );
    json!({ "query": query })
}

/// What GitHub said about the pull requests associated with one commit.
pub enum Lookup {
    Found(Vec<Pr>),
    /// The lookup was made and failed, or GitHub does not know the commit.
    Failed,
    /// The lookup was never made: GitHub was unavailable or collection had stopped.
    Skipped,
}

/// One unit of concurrent collection work.
pub enum Job {
    /// Every published release, to choose the style reference.
    Releases,
    /// One release by tag.
    View(String),
    /// A GraphQL association query for consecutive commits.
    Batch {
        offset: usize,
        total: usize,
        shas: Vec<String>,
    },
    /// All REST association pages for one commit.
    Rest(String),
}

pub enum Done {
    Releases(Result<Vec<Value>, Fetch>),
    View(Fetch),
    /// Per commit: a final lookup, or `None` when it needs the REST listing.
    Batch(Vec<Option<Lookup>>),
    Rest(Lookup),
}

/// A path segment, percent-encoded (tags may contain `/`).
fn segment(text: &str) -> String {
    super::notes::quote_path(text).replace('/', "%2F")
}

impl GitHub {
    /// Runs collection jobs concurrently; results and their diagnostics come back in job
    /// order regardless of completion order.
    pub fn run(&self, slug: &str, jobs: Vec<Job>) -> Vec<(Done, Vec<String>)> {
        run_jobs(CONCURRENCY, jobs, |job| {
            let mut notes = Vec::new();
            let done = match job {
                Job::Releases => Done::Releases(
                    self.get_pages(&format!("repos/{slug}/releases?per_page=100"), &mut notes),
                ),
                Job::View(tag) => Done::View(self.request(
                    &format!("repos/{slug}/releases/tags/{}", segment(&tag)),
                    None,
                    &mut notes,
                )),
                Job::Batch {
                    offset,
                    total,
                    shas,
                } => Done::Batch(self.batch(slug, offset, total, &shas, &mut notes)),
                Job::Rest(sha) => Done::Rest(self.rest(slug, &sha, &mut notes)),
            };
            (done, notes)
        })
    }

    fn batch(
        &self,
        slug: &str,
        offset: usize,
        total: usize,
        shas: &[String],
        notes: &mut Vec<String>,
    ) -> Vec<Option<Lookup>> {
        let all = |lookup: fn() -> Option<Lookup>| shas.iter().map(|_| lookup()).collect();
        let Some(client) = self.client.as_ref().filter(|_| !self.stopped()) else {
            return all(|| Some(Lookup::Skipped));
        };
        eprintln!(
            "info: Resolving PRs for commits {}-{} of {total}",
            offset + 1,
            offset + shas.len()
        );
        let (owner, name) = slug.split_once('/').unwrap_or((slug, ""));
        let query = association_query(owner, name, shas);
        let result = match self.request(&client.graphql_url(), Some(&query), notes) {
            Fetch::Ok(value) => value,
            Fetch::Skipped => return all(|| Some(Lookup::Skipped)),
            // A query or schema failure gets one REST listing per commit; rate limits,
            // hangs, and authentication failures have already opened the circuit.
            Fetch::Failed => return all(|| None),
        };
        let Some(data) = result
            .pointer("/data/repository")
            .filter(|data| data.is_object())
        else {
            return all(|| None);
        };
        (0..shas.len())
            .map(|index| {
                let connection = data
                    .get(format!("c{index}"))
                    .and_then(|commit| commit.get("associatedPullRequests"));
                // A commit GitHub does not know (e.g. unpushed work) remains unknown.
                let Some(nodes) = connection
                    .and_then(|connection| connection.get("nodes"))
                    .and_then(Value::as_array)
                else {
                    return Some(Lookup::Failed);
                };
                let more = |value: &Value| {
                    value.pointer("/pageInfo/hasNextPage") == Some(&Value::Bool(true))
                };
                let overflow = connection.is_some_and(more)
                    || nodes
                        .iter()
                        .any(|node| node.get("labels").is_some_and(more));
                if overflow {
                    return None;
                }
                let prs: Option<Vec<Pr>> = nodes.iter().map(Pr::from_graphql).collect();
                Some(prs.map_or(Lookup::Failed, Lookup::Found))
            })
            .collect()
    }

    /// Every association page for a commit, followed to exhaustion.
    fn rest(&self, slug: &str, sha: &str, notes: &mut Vec<String>) -> Lookup {
        match self.get_pages(
            &format!("repos/{slug}/commits/{sha}/pulls?per_page=100"),
            notes,
        ) {
            Ok(items) => {
                let prs: Option<Vec<Pr>> = items.iter().map(Pr::from_rest).collect();
                prs.map_or(Lookup::Failed, Lookup::Found)
            }
            Err(Fetch::Skipped) => Lookup::Skipped,
            Err(_) => Lookup::Failed,
        }
    }
}
