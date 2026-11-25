//! `pr-diff`: copy a pull request's title, body, URL, and patch.
//!
//! Owned by migration track A. Reference: libexec/pr-diff and focal/gh_pr_diff.py.

use crate::file::fenced;
use crate::github::{self, ApiError, Client, Repo};
use crate::output::{deliver, fail, header};
use crate::picker::{self, Item, Options, Outcome};
use serde_json::Value;
use std::io;

/// Open pull requests offered in the picker, as `gh pr list --limit 30`.
const LIMIT: usize = 30;

#[derive(clap::Args)]
pub struct Args {
    /// Pull request number, URL, or head branch; omit to use the current branch's pull request.
    #[arg(value_name = "PR_ID")]
    pub pr: Option<String>,
}

/// Percent-encodes a query value, keeping the characters a `head` filter needs readable.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'/' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// A number from `123` or `#123`.
fn parse_number(id: &str) -> Option<u64> {
    let id = id.trim();
    id.strip_prefix('#').unwrap_or(id).parse().ok()
}

/// The repository and number named by a pull request URL
/// (`https://host/owner/name/pull/123[/files…]`).
fn parse_url(id: &str) -> Option<(Repo, u64)> {
    let id = id.trim();
    if !(id.starts_with("https://") || id.starts_with("http://")) {
        return None;
    }
    let (repo, rest) = id.split_once("/pull/")?;
    let number = rest.split(['/', '?', '#']).next()?.parse().ok()?;
    Some((github::parse_remote(repo)?, number))
}

/// The pull request whose head is `branch` in this repository, preferring an open one.
fn for_branch(client: &Client, branch: &str) -> Result<Option<u64>, ApiError> {
    let head = encode(&format!("{}:{branch}", client.repo.owner));
    let pulls = client.get_json(&format!(
        "/repos/{}/pulls?head={head}&state=all&per_page=100",
        client.repo.slug()
    ))?;
    let pulls = pulls.as_array().cloned().unwrap_or_default();
    let chosen = pulls
        .iter()
        .find(|pull| pull.get("state").and_then(Value::as_str) == Some("open"))
        .or_else(|| pulls.first());
    Ok(chosen
        .and_then(|pull| pull.get("number"))
        .and_then(Value::as_u64))
}

/// The current branch's upstream branch name (or its own name), as `gh pr view` resolves it.
fn current_branch() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let repo = crate::git::open(&cwd).ok()??;
    let (branch, _) = crate::git::head(&repo).ok()?;
    if branch.is_empty() {
        return None;
    }
    let upstream = repo
        .config()
        .ok()
        .and_then(|config| config.get_string(&format!("branch.{branch}.merge")).ok())
        .and_then(|merge| merge.strip_prefix("refs/heads/").map(str::to_owned));
    Some(upstream.unwrap_or(branch))
}

/// Lets the user choose one open pull request. `None` means cancelled or nothing to pick.
fn choose(client: &Client) -> io::Result<Option<u64>> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|error| {
            fail(
                format!(
                    "no pull request found for the current branch, and choosing one needs an interactive terminal ({error})"
                ),
                "pass a pull request number: focal pr-diff <number>",
            )
        })?;
    let pulls = client.get_json(&format!(
        "/repos/{}/pulls?state=open&per_page={LIMIT}",
        client.repo.slug()
    ))?;
    let pulls: Vec<(u64, String)> = pulls
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pull| {
            let number = pull.get("number")?.as_u64()?;
            let head = pull.pointer("/head/ref").and_then(Value::as_str);
            let title = pull.get("title").and_then(Value::as_str);
            Some((
                number,
                format!(
                    "[{}] {}",
                    head.unwrap_or_default(),
                    title.unwrap_or_default()
                ),
            ))
        })
        .take(LIMIT)
        .collect();
    if pulls.is_empty() {
        eprintln!(
            "info: No open pull requests found in {}.",
            client.repo.slug()
        );
        return Ok(None);
    }
    let width = pulls
        .iter()
        .map(|(number, _)| number.to_string().len())
        .max()
        .unwrap_or(0);
    let items = pulls
        .iter()
        .map(|(number, text)| {
            let label = format!("{number:<width$}  {text}");
            Item {
                search: label.clone(),
                label,
                weight: 0,
            }
        })
        .collect();
    let options = Options {
        prompt: "Select PR: ".into(),
        header: Vec::new(),
        multi: false,
    };
    Ok(match picker::pick(items, &options, None)? {
        Outcome::Selected(ids) => ids.first().map(|&id| pulls[id].0),
        Outcome::Cancelled => None,
    })
}

/// Resolves an explicit identifier: a number, `#number`, URL, or head branch name.
fn resolve(client: &Client, id: &str) -> io::Result<u64> {
    if let Some(number) = parse_number(id) {
        return Ok(number);
    }
    for_branch(client, id)?.ok_or_else(|| {
        fail(
            format!("no pull request found for '{id}'"),
            "pass a pull request number, URL, or head branch name",
        )
    })
}

fn render(client: &Client, number: u64) -> Result<String, ApiError> {
    let path = format!("/repos/{}/pulls/{number}", client.repo.slug());
    let pull = client.get_json(&path)?;
    let diff = client
        .get_text(&path, "application/vnd.github.diff")
        .map_err(|error| ApiError {
            hint: "verify that the PR exists and you have access".into(),
            ..error
        })?;
    let text = |key: &str| {
        pull.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let title = pull
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let body = text("body");
    let parts = [
        format!("# PR #{number}: {title}"),
        format!("URL: {}\n", text("html_url")),
        "## Intent / Description".to_owned(),
        if body.is_empty() {
            "*No description provided.*".to_owned()
        } else {
            body
        },
        "\n## Diff".to_owned(),
        fenced(diff.trim(), "diff"),
    ];
    Ok(parts.join("\n"))
}

/// A client for the repository a pull request URL names, which may differ from `origin`
/// (as with `gh pr view <url>`).
fn client_for(repo: Repo) -> io::Result<Client> {
    let token = github::token(&repo.host)?;
    Ok(Client::new(repo, token, github::DEFAULT_TIMEOUT))
}

pub fn run(args: Args) -> io::Result<()> {
    if let Some((repo, number)) = args.pr.as_deref().and_then(parse_url) {
        let client = client_for(repo)?;
        return copy(&client, number);
    }
    let client = Client::for_current_repo()?;
    let number = match args.pr.as_deref() {
        Some(id) => resolve(&client, id)?,
        None => {
            // Like `gh pr view 2>/dev/null`, a failed lookup quietly falls back to the picker.
            let found = current_branch().and_then(|branch| for_branch(&client, &branch).ok()?);
            match found {
                Some(number) => number,
                None => match choose(&client)? {
                    Some(number) => number,
                    None => return Ok(()),
                },
            }
        }
    };
    copy(&client, number)
}

fn copy(client: &Client, number: u64) -> io::Result<()> {
    let formatted = render(client, number)?;
    let mut payload = header()?;
    payload.push_str(&formatted);
    deliver(
        payload,
        &format!("Copied context and diff for PR #{number}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identifiers() {
        assert_eq!(parse_number("42"), Some(42));
        assert_eq!(parse_number("#42"), Some(42));
        assert_eq!(parse_number("feature/login"), None);
        let (repo, number) = parse_url("https://github.com/other/lib/pull/42/files").unwrap();
        assert_eq!((repo.slug().as_str(), number), ("other/lib", 42));
        // Branch names that merely contain /pull/ are not URLs.
        assert_eq!(parse_url("release/pull/3"), None);
        assert_eq!(parse_number("release/pull/3"), None);
    }

    #[test]
    fn encodes_head_filters() {
        assert_eq!(encode("octo:feat/a+b c"), "octo:feat/a%2Bb%20c");
    }
}
