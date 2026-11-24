//! Binary downloads for `ci-fail` (migration track B).
//!
//! The run and job logs endpoints answer with a redirect to a pre-signed archive URL on
//! another host. The agent follows it but never forwards the `Authorization` header across
//! a redirect, so the token stays with the API host.

use super::{ApiError, Client, api_message};

impl Client {
    /// GETs a response body as bytes, following redirects without credentials. Statuses
    /// outside 2xx become an [`ApiError`] carrying the final status.
    pub fn download(&self, path_or_url: &str, limit: u64) -> Result<Vec<u8>, ApiError> {
        let url = self.url(path_or_url);
        let mut response = self
            .agent
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|error| ApiError {
                status: None,
                rate_limited: false,
                message: format!("could not download {url}: {error}"),
                hint: "check network connectivity".into(),
            })?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|error| ApiError {
                status: Some(status),
                rate_limited: false,
                message: format!("could not read the download from {url}: {error}"),
                hint: "retry the command".into(),
            })?;
        if (200..300).contains(&status) {
            return Ok(body);
        }
        let text = String::from_utf8_lossy(&body);
        let detail = api_message(&text).unwrap_or_else(|| text.trim().chars().take(200).collect());
        let (message, hint) = match status {
            401 => (
                format!("GitHub rejected the credentials (HTTP 401): {detail}"),
                "refresh GH_TOKEN/GITHUB_TOKEN or run 'gh auth login'",
            ),
            403 => (
                format!("GitHub denied the download (HTTP 403): {detail}"),
                "check that the token can read this repository's Actions logs",
            ),
            _ => (
                format!("download failed (HTTP {status}): {url}"),
                "retry the command; GitHub may be unavailable",
            ),
        };
        Err(ApiError {
            status: Some(status),
            rate_limited: status == 429,
            message,
            hint: hint.into(),
        })
    }
}
