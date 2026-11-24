//! Minimal synchronous GitHub REST client shared by `issues`, `pr-diff`, `ci-fail`, and
//! `release-context`.
//!
//! The repository comes from the current checkout's `origin` remote (which is the cached
//! checkout under `--repo`). Credentials come from `GH_TOKEN`/`GITHUB_TOKEN` (or the
//! enterprise variants for other hosts), falling back to `gh auth token` when GitHub CLI
//! is installed. `FOCAL_GITHUB_API_URL` overrides the API base, which the tests use to
//! point the client at a local fixture server.

// Remove once the GitHub commands use the client (migration tracks A-C).
#![allow(dead_code)]

use serde_json::Value;
use std::fmt;
use std::io;
use std::process::{Command, Stdio};
use std::time::Duration;
use ureq::config::RedirectAuthHeaders;

mod track_b;

/// Default per-request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
/// Upper bound on pages followed by [`Client::get_all`], as a runaway guard.
const MAX_PAGES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// `owner/name`, as used in API paths.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Parses HTTPS, `ssh://`, and scp-style (`git@host:owner/name.git`) remote URLs.
pub fn parse_remote(url: &str) -> Option<Repo> {
    let url = url.trim();
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let (authority, path) = rest.split_once('/')?;
        let host = authority.split(':').next()?;
        (host, path)
    } else {
        let rest = url.rsplit_once('@').map_or(url, |(_, r)| r);
        rest.split_once(':')?
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.rsplitn(2, '/');
    let name = parts.next()?;
    let owner = parts.next()?.rsplit('/').next()?;
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(Repo {
        host: host.to_ascii_lowercase(),
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// The GitHub repository behind the current checkout's `origin` remote.
pub fn current_repo() -> io::Result<Repo> {
    let cwd = std::env::current_dir()?;
    let repo = crate::git::open(&cwd)
        .map_err(io::Error::other)?
        .ok_or_else(|| {
            crate::output::fail(
                "not a git repository",
                "run this inside a GitHub checkout, or pass --repo <url>",
            )
        })?;
    let remote = repo.find_remote("origin").map_err(|_| {
        crate::output::fail(
            "this repository has no 'origin' remote",
            "add the GitHub remote: git remote add origin <url>",
        )
    })?;
    let url = remote.url().unwrap_or_default();
    parse_remote(url).ok_or_else(|| {
        crate::output::fail(
            format!("could not read a GitHub owner and repository from origin '{url}'"),
            "point origin at a GitHub repository URL",
        )
    })
}

fn env_token(names: &[&str]) -> Option<String> {
    names
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|value| value.trim().to_owned())
        .find(|value| !value.is_empty())
}

/// Finds a token for `host`: environment variables first, then `gh auth token`.
pub fn token(host: &str) -> io::Result<String> {
    let names: &[&str] = if host == "github.com" {
        &["GH_TOKEN", "GITHUB_TOKEN"]
    } else {
        &[
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
            "GH_TOKEN",
            "GITHUB_TOKEN",
        ]
    };
    if let Some(token) = env_token(names) {
        return Ok(token);
    }
    let from_gh = Command::new("gh")
        .args(["auth", "token", "--hostname", host])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|token| !token.is_empty());
    from_gh.ok_or_else(|| {
        crate::output::fail(
            format!("no GitHub credentials found for {host}"),
            "set GH_TOKEN or GITHUB_TOKEN, or sign in with 'gh auth login'",
        )
    })
}

/// A failed API call, with enough structure for callers to report per-item failures or
/// tell a rate limit apart from a missing resource.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// HTTP status, or `None` for transport failures (DNS, timeout, TLS).
    pub status: Option<u16>,
    pub rate_limited: bool,
    pub message: String,
    pub hint: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n  hint: {}", self.message, self.hint)
    }
}

impl std::error::Error for ApiError {}

impl From<ApiError> for io::Error {
    fn from(error: ApiError) -> Self {
        io::Error::other(error.to_string())
    }
}

fn api_message(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .get("message")?
        .as_str()
        .map(str::to_owned)
}

/// Extracts the `rel="next"` target from a `Link` header.
pub fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        params
            .split(';')
            .any(|param| param.trim() == "rel=\"next\"")
            .then(|| {
                target
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_owned()
            })
    })
}

pub struct Client {
    agent: ureq::Agent,
    api: String,
    token: String,
    pub repo: Repo,
}

/// A successful response: status, body, and the next-page link when present.
pub struct Page {
    pub body: String,
    pub next: Option<String>,
}

impl Client {
    /// A client for the current checkout's repository, with credentials resolved.
    pub fn for_current_repo() -> io::Result<Self> {
        let repo = current_repo()?;
        let token = token(&repo.host)?;
        Ok(Self::new(repo, token, DEFAULT_TIMEOUT))
    }

    pub fn new(repo: Repo, token: String, timeout: Duration) -> Self {
        let api = std::env::var("FOCAL_GITHUB_API_URL")
            .ok()
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| {
                if repo.host == "github.com" {
                    "https://api.github.com".into()
                } else {
                    format!("https://{}/api/v3", repo.host)
                }
            });
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .redirect_auth_headers(RedirectAuthHeaders::Never)
            .user_agent("focal")
            .build()
            .new_agent();
        Self {
            agent,
            api: api.trim_end_matches('/').to_owned(),
            token,
            repo,
        }
    }

    /// Resolves an API path (`/repos/o/r/issues?...`) or passes an absolute URL through.
    pub fn url(&self, path_or_url: &str) -> String {
        if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            path_or_url.to_owned()
        } else {
            format!("{}/{}", self.api, path_or_url.trim_start_matches('/'))
        }
    }

    /// Performs a GET with the given `Accept` header and maps failures to [`ApiError`].
    pub fn request(&self, path_or_url: &str, accept: &str) -> Result<Page, ApiError> {
        let url = self.url(path_or_url);
        let response = self
            .agent
            .get(&url)
            .header("Accept", accept)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call();
        let mut response = match response {
            Ok(response) => response,
            Err(error) => {
                return Err(ApiError {
                    status: None,
                    rate_limited: false,
                    message: format!("could not reach the GitHub API at {url}: {error}"),
                    hint: "check network connectivity".into(),
                });
            }
        };
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let next = header("link").as_deref().and_then(next_link);
        let remaining = header("x-ratelimit-remaining");
        let reset = header("x-ratelimit-reset");
        let body = response
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_string()
            .map_err(|error| ApiError {
                status: Some(status),
                rate_limited: false,
                message: format!("could not read the GitHub response from {url}: {error}"),
                hint: "retry the command".into(),
            })?;
        if (200..300).contains(&status) {
            return Ok(Page { body, next });
        }
        let detail = api_message(&body).unwrap_or_else(|| body.trim().chars().take(200).collect());
        let rate_limited = (status == 403 || status == 429)
            && (remaining.as_deref() == Some("0") || detail.to_lowercase().contains("rate limit"));
        let (message, hint) = match status {
            401 => (
                format!("GitHub rejected the credentials (HTTP 401): {detail}"),
                "refresh GH_TOKEN/GITHUB_TOKEN or run 'gh auth login'".to_owned(),
            ),
            _ if rate_limited => (
                format!("GitHub API rate limit exceeded (HTTP {status})"),
                match reset.and_then(|r| r.parse::<u64>().ok()) {
                    Some(epoch) => {
                        format!("wait until the limit resets (Unix time {epoch}) and retry")
                    }
                    None => "wait a few minutes and retry".to_owned(),
                },
            ),
            403 => (
                format!("GitHub denied access (HTTP 403): {detail}"),
                "check that the token can read this repository".to_owned(),
            ),
            404 => (
                format!("not found on GitHub (HTTP 404): {url}"),
                "check the repository and number, and that the token can access it".to_owned(),
            ),
            _ => (
                format!("GitHub API error (HTTP {status}): {detail}"),
                "retry the command; GitHub may be unavailable".to_owned(),
            ),
        };
        Err(ApiError {
            status: Some(status),
            rate_limited,
            message,
            hint,
        })
    }

    /// GETs one JSON document.
    pub fn get_json(&self, path_or_url: &str) -> Result<Value, ApiError> {
        let page = self.request(path_or_url, "application/vnd.github+json")?;
        serde_json::from_str(&page.body).map_err(|error| ApiError {
            status: None,
            rate_limited: false,
            message: format!("GitHub returned invalid JSON for {path_or_url}: {error}"),
            hint: "retry the command".into(),
        })
    }

    /// GETs a JSON array and every following page, concatenated.
    pub fn get_all(&self, path_or_url: &str) -> Result<Vec<Value>, ApiError> {
        let mut items = Vec::new();
        let mut next = Some(path_or_url.to_owned());
        for _ in 0..MAX_PAGES {
            let Some(url) = next.take() else { break };
            let page = self.request(&url, "application/vnd.github+json")?;
            match serde_json::from_str::<Value>(&page.body) {
                Ok(Value::Array(values)) => items.extend(values),
                _ => {
                    return Err(ApiError {
                        status: None,
                        rate_limited: false,
                        message: format!("GitHub returned an unexpected page for {url}"),
                        hint: "retry the command".into(),
                    });
                }
            }
            next = page.next;
        }
        Ok(items)
    }

    /// GETs a text representation, such as a PR patch (`application/vnd.github.diff`).
    pub fn get_text(&self, path_or_url: &str, accept: &str) -> Result<String, ApiError> {
        self.request(path_or_url, accept).map(|page| page.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn parses_remote_url_forms() {
        let expected = Repo {
            host: "github.com".into(),
            owner: "octo".into(),
            name: "focal".into(),
        };
        for url in [
            "https://github.com/octo/focal.git",
            "https://github.com/octo/focal",
            "https://user:token@github.com/octo/focal/",
            "git@github.com:octo/focal.git",
            "ssh://git@github.com/octo/focal.git",
            "ssh://git@GitHub.com:22/octo/focal",
        ] {
            assert_eq!(parse_remote(url).as_ref(), Some(&expected), "{url}");
        }
        let enterprise = parse_remote("https://ghe.example.com/team/tool.git").unwrap();
        assert_eq!(
            (enterprise.host.as_str(), enterprise.slug().as_str()),
            ("ghe.example.com", "team/tool")
        );
        assert_eq!(parse_remote("/local/path/repo"), None);
    }

    #[test]
    fn reads_next_links() {
        let header = r#"<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last""#;
        assert_eq!(
            next_link(header).as_deref(),
            Some("https://api.github.com/x?page=2")
        );
        assert_eq!(next_link(r#"<https://a/x?page=1>; rel="prev""#), None);
    }

    /// Serves canned responses in order, one per connection, and records request lines.
    fn serve(
        responses: impl FnOnce(&str) -> Vec<String>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let responses = responses(&base);
        let handle = thread::spawn(move || {
            let mut seen = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut auth = String::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.to_ascii_lowercase().starts_with("authorization:") {
                        auth = header.trim().to_owned();
                    }
                    if header == "\r\n" || header.is_empty() {
                        break;
                    }
                }
                seen.push(format!("{} | {auth}", line.trim()));
                stream.write_all(response.as_bytes()).unwrap();
            }
            seen
        });
        (base, handle)
    }

    fn response(status: &str, headers: &[&str], body: &str) -> String {
        let mut text = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for header in headers {
            text.push_str(header);
            text.push_str("\r\n");
        }
        text + "\r\n" + body
    }

    fn client(base: &str) -> Client {
        let repo = parse_remote("https://github.com/octo/focal").unwrap();
        let mut client = Client::new(repo, "secret".into(), Duration::from_secs(5));
        client.api = base.to_owned();
        client
    }

    #[test]
    fn follows_pagination_with_credentials() {
        let (base, server) = serve(|base| {
            vec![
                response(
                    "200 OK",
                    &[&format!(
                        r#"Link: <{base}/items?page=2>; rel="next", <{base}/items?page=2>; rel="last""#
                    )],
                    "[1,2]",
                ),
                response(
                    "200 OK",
                    &[&format!(r#"Link: <{base}/items?page=1>; rel="prev""#)],
                    "[3]",
                ),
            ]
        });
        let items = client(&base).get_all("/items?per_page=2").unwrap();
        assert_eq!(items, vec![Value::from(1), Value::from(2), Value::from(3)]);
        let seen = server.join().unwrap();
        assert_eq!(
            seen,
            [
                "GET /items?per_page=2 HTTP/1.1 | authorization: Bearer secret",
                "GET /items?page=2 HTTP/1.1 | authorization: Bearer secret",
            ]
        );
    }

    #[test]
    fn maps_error_statuses() {
        let (base, server) = serve(|_| {
            vec![
                response("401 Unauthorized", &[], r#"{"message":"Bad credentials"}"#),
                response(
                    "403 Forbidden",
                    &["x-ratelimit-remaining: 0", "x-ratelimit-reset: 1700000000"],
                    r#"{"message":"API rate limit exceeded"}"#,
                ),
                response("404 Not Found", &[], r#"{"message":"Not Found"}"#),
                response("500 Internal Server Error", &[], "boom"),
            ]
        });
        let client = client(&base);
        let unauthorized = client.get_json("/a").unwrap_err();
        assert_eq!(unauthorized.status, Some(401));
        assert!(unauthorized.message.contains("Bad credentials"));
        let limited = client.get_json("/b").unwrap_err();
        assert!(limited.rate_limited);
        assert!(limited.hint.contains("1700000000"));
        let missing = client.get_json("/c").unwrap_err();
        assert_eq!(missing.status, Some(404));
        assert!(!missing.rate_limited);
        let broken = client.get_json("/d").unwrap_err();
        assert!(broken.message.contains("HTTP 500") && broken.message.contains("boom"));
        server.join().unwrap();
    }
}
