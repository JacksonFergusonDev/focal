//! Client additions for `release-context` (migration track C): a per-request timeout,
//! POST bodies for GraphQL, and the raw status details the release collector needs to
//! classify failures itself (timeouts, rate limits, and service errors).

use super::{Client, api_message};
use std::time::Duration;

/// A completed HTTP exchange, successful or not.
pub struct Reply {
    pub status: u16,
    pub body: String,
    /// The `rel="next"` target from the `Link` header.
    pub next: Option<String>,
    /// `x-ratelimit-remaining: 0` or a `retry-after` header (secondary rate limit).
    pub limited: bool,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The API's `message`, or the start of the body.
    pub fn detail(&self) -> String {
        api_message(&self.body).unwrap_or_else(|| self.body.trim().chars().take(200).collect())
    }
}

/// A request that produced no HTTP response.
pub enum CallError {
    Timeout,
    Transport(String),
}

impl Client {
    /// The GraphQL endpoint next to the REST base (`/api/graphql` on GitHub Enterprise).
    pub fn graphql_url(&self) -> String {
        match self.api.strip_suffix("/api/v3") {
            Some(base) => format!("{base}/api/graphql"),
            None => format!("{}/graphql", self.api),
        }
    }

    /// Performs one GET (no body) or POST (JSON body) bounded by `timeout`, which covers
    /// connecting, sending, and reading the whole response.
    pub fn call(
        &self,
        path_or_url: &str,
        json: Option<&str>,
        timeout: Duration,
    ) -> Result<Reply, CallError> {
        let url = self.url(path_or_url);
        let auth = format!("Bearer {}", self.token);
        let response = match json {
            None => self
                .agent
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("Authorization", &auth)
                .header("X-GitHub-Api-Version", "2022-11-28")
                .config()
                .timeout_global(Some(timeout))
                .build()
                .call(),
            Some(body) => self
                .agent
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("Authorization", &auth)
                .header("Content-Type", "application/json")
                .config()
                .timeout_global(Some(timeout))
                .build()
                .send(body),
        };
        let failure = |error: ureq::Error| match error {
            ureq::Error::Timeout(_) => CallError::Timeout,
            error => CallError::Transport(format!("could not reach the GitHub API: {error}")),
        };
        let mut response = response.map_err(failure)?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let next = header("link").as_deref().and_then(super::next_link);
        let limited = header("x-ratelimit-remaining").as_deref() == Some("0")
            || header("retry-after").is_some();
        let body = response
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_string()
            .map_err(failure)?;
        Ok(Reply {
            status,
            body,
            next,
            limited,
        })
    }
}
