//! `web` contracts.
//!
//! A local HTTP server stands in for real sites. Assertions check content and structure
//! rather than exact Markdown.

mod common;

use common::*;
use regex::Regex;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Output;
use std::sync::OnceLock;
use std::thread;

const PAGE: &str = r#"<!doctype html><html><head><title>Fixture</title></head><body>
<header><a href="/">Site header link</a></header>
<nav><ul><li><a href="/a">Navigation item</a></li></ul></nav>
<main>
<h1>Main heading</h1>
<p>Body paragraph with <strong>bold</strong> text.</p>
<table><tr><th>Name</th><th>Value</th></tr><tr><td>alpha</td><td>1</td></tr></table>
<pre><code>print("code block")</code></pre>
<form><button>Submit button</button></form>
<svg><text>Vector label</text></svg>
</main>
<aside>Sidebar aside</aside>
<footer>Footer text</footer>
</body></html>"#;

const ERRORS: [(u16, &str); 5] = [
    (401, "Unauthorized"),
    (403, "Forbidden"),
    (404, "Not Found"),
    (999, "Denied"),
    (500, "Oops"),
];

/// The base URL of a local server shared by every test in this file.
fn server() -> &'static str {
    static URL: OnceLock<String> = OnceLock::new();
    URL.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind server");
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                thread::spawn(move || handle(stream));
            }
        });
        url
    })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "",
    }
}

fn handle(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_owned();
    let mut accept_encoding = String::new();
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line.trim_end().is_empty() => break,
            Ok(_) => {
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("accept-encoding")
                {
                    accept_encoding = value.trim().to_owned();
                }
            }
        }
    }
    let html = [("Content-Type", "text/html")];
    let (status, body, headers): (u16, Vec<u8>, Vec<(&str, &str)>) = match path.as_str() {
        "/page" => (200, PAGE.into(), html.to_vec()),
        "/redirect" => (302, Vec::new(), vec![("Location", "/page")]),
        "/gzip" if accept_encoding.contains("gzip") => (
            200,
            gzip(PAGE.as_bytes()),
            vec![("Content-Type", "text/html"), ("Content-Encoding", "gzip")],
        ),
        "/long" => (
            200,
            format!("<p>{}</p>", "word ".repeat(12_000)).into(),
            html.to_vec(),
        ),
        "/empty" => (200, Vec::new(), Vec::new()),
        other => match ERRORS
            .iter()
            .find(|(code, _)| other.strip_prefix('/') == Some(&code.to_string()))
        {
            Some((code, message)) => (*code, message.as_bytes().to_vec(), Vec::new()),
            None => (400, b"unexpected".to_vec(), Vec::new()),
        },
    };
    let mut response = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

/// A gzip member holding `data` in stored (uncompressed) deflate blocks.
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let mut chunks = data.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn base_env(tmp: &Path) -> Env {
    env(&[
        ("PATH", ""),
        ("HOME", tmp.to_str().unwrap()),
        ("LC_ALL", "C.UTF-8"),
        ("NO_COLOR", "1"),
    ])
}

fn run(tmp: &Path, args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut full = vec!["web"];
    full.extend_from_slice(args);
    focal(tmp, &base_env(tmp), &full, stdin)
}

fn check_main_content(markdown: &str) {
    assert!(markdown.contains("Main heading"), "{markdown}");
    assert!(markdown.contains("**bold**"), "{markdown}");
    assert!(
        Regex::new(r"(?m)^\| Name +\| Value +\|$")
            .unwrap()
            .is_match(markdown),
        "{markdown}"
    );
    assert!(
        Regex::new(r"(?m)^\| alpha +\| 1 +\|$")
            .unwrap()
            .is_match(markdown),
        "{markdown}"
    );
    assert!(markdown.contains(r#"print("code block")"#), "{markdown}");
    for excluded in [
        "Site header link",
        "Navigation item",
        "Submit button",
        "Vector label",
        "Sidebar aside",
        "Footer text",
    ] {
        assert!(!markdown.contains(excluded), "{excluded}: {markdown}");
    }
}

fn check_fetched_page(path: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}{path}", server());
    let result = run(tmp.path(), &[&url], None);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    assert!(
        out.starts_with(&format!("<context>\n# Source: {url}\n\n")),
        "{out}"
    );
    assert!(!out.contains("Workspace Context"));
    assert!(out.ends_with("\n</context>\n"), "{out}");
    check_main_content(&out);
    let err = text(&result.stderr);
    assert!(
        err.starts_with(&format!("done: Copied markdown context for {url} (~")),
        "{err}"
    );
}

#[test]
fn fetched_page_page() {
    check_fetched_page("/page");
}

#[test]
fn fetched_page_redirect() {
    check_fetched_page("/redirect");
}

#[test]
fn fetched_page_gzip() {
    check_fetched_page("/gzip");
}

#[test]
fn piped_html() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &[], Some(PAGE.as_bytes()));
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    assert!(
        out.starts_with("<context>\n# Source: Piped DOM/Clipboard\n\n"),
        "{out}"
    );
    check_main_content(&out);
    assert!(text(&result.stderr).contains("done: Copied markdown context from piped HTML (~"));
}

#[test]
fn output_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &[&format!("{}/long", server())], None);
    assert_eq!(result.status.code(), Some(0), "{}", text(&result.stderr));
    let out = text(&result.stdout);
    assert!(
        out.ends_with("\n\n...[web content truncated: exceeded 50000 characters]\n</context>\n"),
        "{}",
        &out[out.len().saturating_sub(200)..]
    );
    let body = out
        .split_once("\n\n")
        .unwrap()
        .1
        .split("\n\n...[web content truncated")
        .next()
        .unwrap();
    // Python's len() counts characters.
    assert_eq!(body.chars().count(), 50_000);
}

fn check_http_error(status: u16, message: &str, hint: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &[&format!("{}/{status}", server())], None);
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(text(&result.stdout), "");
    let err = text(&result.stderr);
    assert!(err.contains(&format!("error: {message}")), "{err}");
    let hint_line = err
        .lines()
        .find(|line| line.trim_start().starts_with("hint:"))
        .unwrap_or_else(|| panic!("no hint line: {err}"));
    assert!(hint_line.contains(hint), "{hint_line}");
}

#[test]
fn http_errors_401() {
    check_http_error(
        401,
        "HTTP 401 Unauthorized returned by 127.0.0.1",
        "requires authentication",
    );
}

#[test]
fn http_errors_403() {
    check_http_error(
        403,
        "HTTP 403 Forbidden returned by 127.0.0.1",
        "pbpaste | focal web",
    );
}

#[test]
fn http_errors_404() {
    check_http_error(
        404,
        "HTTP 404 Not Found returned for ",
        "URL path is spelled correctly",
    );
}

#[test]
fn http_errors_999() {
    check_http_error(
        999,
        "HTTP 999 returned by 127.0.0.1 (anti-bot",
        "blocks automated CLI scrapers",
    );
}

#[test]
fn http_errors_500() {
    check_http_error(500, "HTTP 500 returned for ", "check network connectivity");
}

#[test]
fn empty_response() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &[&format!("{}/empty", server())], None);
    assert_eq!(result.status.code(), Some(1));
    assert!(text(&result.stderr).contains("empty response received"));
}

#[test]
fn connection_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &["http://127.0.0.1:9/"], None);
    assert_eq!(result.status.code(), Some(1));
    let err = text(&result.stderr);
    assert!(
        err.contains("error: Error fetching http://127.0.0.1:9/: "),
        "{err}"
    );
    assert!(
        err.contains("hint: check network connectivity or URL validity"),
        "{err}"
    );
}

#[test]
fn invalid_scheme() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &["ftp://example.com/file"], None);
    assert_eq!(result.status.code(), Some(1));
    let err = text(&result.stderr);
    assert!(
        err.contains("error: Invalid URL scheme in 'ftp://example.com/file'"),
        "{err}"
    );
    assert!(
        err.contains("hint: URL must begin with http:// or https://"),
        "{err}"
    );
}

#[test]
fn empty_stdin() {
    let tmp = tempfile::tempdir().unwrap();
    let result = run(tmp.path(), &[], Some(b"  \n\t\n"));
    assert_eq!(result.status.code(), Some(1));
    assert!(text(&result.stderr).contains("error: received empty piped input"));
}
