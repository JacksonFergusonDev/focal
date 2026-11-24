//! `web`: fetch a page (or read piped browser HTML) and convert its main content to Markdown.

use html_to_markdown_rs::{ConversionOptions, PreprocessingOptions, convert};
use std::io::{self, IsTerminal, Read};
use std::time::Duration;

const MAX_OUTPUT_CHARS: usize = 50_000;
const EXCLUDED: [&str; 7] = ["nav", "footer", "header", "aside", "button", "form", "svg"];
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const HEADERS: [(&str, &str); 10] = [
    (
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
    ),
    ("Accept-Language", "en-US,en;q=0.9"),
    (
        "Sec-Ch-Ua",
        "\"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\"",
    ),
    ("Sec-Ch-Ua-Mobile", "?0"),
    ("Sec-Ch-Ua-Platform", "\"macOS\""),
    ("Sec-Fetch-Dest", "document"),
    ("Sec-Fetch-Mode", "navigate"),
    ("Sec-Fetch-Site", "none"),
    ("Sec-Fetch-User", "?1"),
    ("Upgrade-Insecure-Requests", "1"),
];

pub const HELP: &str = "Extracts semantic markdown from public URLs or authenticated piped HTML.

Usage: focal web <url> | pbpaste | focal web

Examples:
  focal web https://docs.python.org/3/      # Fetch a public documentation page
  pbpaste | focal web                       # Parse clipboard HTML content";

fn fail(message: String, hint: &str) -> io::Error {
    io::Error::other(format!("{message}\n  hint: {hint}"))
}

/// Converts HTML with the same settings as `html-to-markdown --preprocess --exclude-selectors …`.
fn markdown(html: &str) -> Result<String, String> {
    let options = ConversionOptions {
        preprocessing: PreprocessingOptions {
            enabled: true,
            ..PreprocessingOptions::default()
        },
        exclude_selectors: EXCLUDED.iter().map(|s| (*s).to_owned()).collect(),
        extract_metadata: false,
        ..ConversionOptions::default()
    };
    let result = convert(html, options).map_err(|error| error.to_string())?;
    Ok(result.content.unwrap_or_default())
}

/// Applies the 50,000-character cap and the trailing truncation notice.
fn body(markdown: &str) -> String {
    let text = markdown.trim_end_matches('\n');
    if text.chars().count() > MAX_OUTPUT_CHARS {
        let cut: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
        format!("{cut}\n\n...[web content truncated: exceeded {MAX_OUTPUT_CHARS} characters]")
    } else {
        text.to_owned()
    }
}

/// Adds `https://` to a bare host and rejects non-HTTP schemes.
fn normalize(url: &str) -> io::Result<String> {
    let scheme = url.split_once("://").map(|(scheme, _)| scheme);
    let has_scheme = scheme.is_some_and(|s| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    });
    if !has_scheme {
        return Ok(format!("https://{url}"));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url.to_owned())
    } else {
        Err(fail(
            format!("Invalid URL scheme in '{url}'"),
            "URL must begin with http:// or https://",
        ))
    }
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let end = rest.find(['/', ':', '?', '#']).unwrap_or(rest.len());
    &rest[..end]
}

/// Fetches a page with browser-like headers, following redirects, within 20 seconds.
fn fetch(url: &str) -> io::Result<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .build()
        .new_agent();
    let mut request = agent.get(url);
    for (name, value) in HEADERS {
        request = request.header(name, value);
    }
    let connectivity = "check network connectivity or URL validity";
    let mut response = request
        .call()
        .map_err(|error| fail(format!("Error fetching {url}: {error}"), connectivity))?;
    let status = response.status().as_u16();
    let site = host(url);
    let site = if site.is_empty() { url } else { site };
    match status {
        999 => Err(fail(
            format!("HTTP 999 returned by {site} (anti-bot protection or login wall)"),
            "this site blocks automated CLI scrapers. Open the page in your browser and run: pbpaste | focal web",
        )),
        403 => Err(fail(
            format!("HTTP 403 Forbidden returned by {site} (access denied or bot protection)"),
            "if this page requires login or interactive verification, open it in your browser and run: pbpaste | focal web",
        )),
        401 => Err(fail(
            format!("HTTP 401 Unauthorized returned by {site}"),
            "this page requires authentication. Log in via your browser, copy the page, and run: pbpaste | focal web",
        )),
        404 => Err(fail(
            format!("HTTP 404 Not Found returned for {url}"),
            "check that the URL path is spelled correctly",
        )),
        200..=399 => {
            let bytes = response
                .body_mut()
                .with_config()
                .limit(64 * 1024 * 1024)
                .read_to_vec()
                .map_err(|error| fail(format!("Error fetching {url}: {error}"), connectivity))?;
            if bytes.is_empty() {
                return Err(fail(
                    format!("Error fetching {url}: empty response received"),
                    "check that the server returned content",
                ));
            }
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => Err(fail(
            format!("HTTP {status} returned for {url}"),
            connectivity,
        )),
    }
}

/// The section body and done message for a URL, piped HTML, or `None` when there is no input.
pub fn render(url: Option<&str>) -> io::Result<Option<(String, String)>> {
    if let Some(url) = url {
        let url = normalize(url)?;
        let html = fetch(&url)?;
        let md = markdown(&html).map_err(|error| {
            fail(
                format!("failed to parse HTML from {url}: {error}"),
                "the webpage returned invalid HTML content",
            )
        })?;
        return Ok(Some((
            format!("# Source: {url}\n\n{}", body(&md)),
            format!("Copied markdown context for {url}"),
        )));
    }
    if io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut raw = Vec::new();
    io::stdin().read_to_end(&mut raw)?;
    let html = String::from_utf8_lossy(&raw);
    if html.trim().is_empty() {
        return Err(fail(
            "received empty piped input".into(),
            "pipe HTML via stdin or provide a URL argument",
        ));
    }
    let md = markdown(&html).map_err(|error| {
        fail(
            format!("failed to parse HTML input: {error}"),
            "ensure input is valid HTML",
        )
    })?;
    Ok(Some((
        format!("# Source: Piped DOM/Clipboard\n\n{}", body(&md)),
        "Copied markdown context from piped HTML".into(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls() {
        assert_eq!(normalize("example.com/a").unwrap(), "https://example.com/a");
        assert_eq!(normalize("http://x.test").unwrap(), "http://x.test");
        assert!(normalize("ftp://x.test").is_err());
        assert_eq!(host("https://docs.python.org:443/3/"), "docs.python.org");
    }

    #[test]
    fn truncates_by_characters() {
        let long = "é".repeat(MAX_OUTPUT_CHARS + 5);
        let out = body(&long);
        assert!(out.ends_with("\n\n...[web content truncated: exceeded 50000 characters]"));
        assert_eq!(out.chars().filter(|&c| c == 'é').count(), MAX_OUTPUT_CHARS);
    }
}
