//! `issues`: list open (or all) issues, let the user pick, and copy each with its comments.
//!
//! Owned by migration track A. Reference: libexec/issues and focal/gh_issues.py.

use crate::github::{ApiError, Client};
use crate::output::{deliver, header};
use crate::picker::{self, Item, Options, Outcome};
use serde_json::Value;
use std::io;
use std::sync::Arc;

/// Issues offered in the picker, as `gh issue list --limit 100`.
const LIMIT: usize = 100;
/// Listing pages followed before giving up, since pull requests share the issues listing.
const MAX_LIST_PAGES: usize = 20;

#[derive(clap::Args)]
pub struct Args {
    /// Show both open and closed issues.
    #[arg(long)]
    pub all: bool,
}

/// One row of the issue listing.
struct Summary {
    number: u64,
    state: String,
    title: String,
    author: String,
    body: String,
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn login(value: &Value) -> String {
    value
        .pointer("/user/login")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_owned()
}

/// Lists up to [`LIMIT`] issues, newest first, skipping the pull requests that the issues
/// endpoint also returns.
fn list(client: &Client, all: bool) -> Result<Vec<Summary>, ApiError> {
    let state = if all { "all" } else { "open" };
    let mut next = Some(format!(
        "/repos/{}/issues?state={state}&per_page={LIMIT}",
        client.repo.slug()
    ));
    let mut issues = Vec::new();
    for _ in 0..MAX_LIST_PAGES {
        let Some(url) = next.take() else { break };
        let page = client.request(&url, "application/vnd.github+json")?;
        let Ok(Value::Array(values)) = serde_json::from_str::<Value>(&page.body) else {
            return Err(ApiError {
                status: None,
                rate_limited: false,
                message: format!("GitHub returned an unexpected issue listing for {url}"),
                hint: "retry the command".into(),
            });
        };
        for value in values {
            if value.get("pull_request").is_some() {
                continue;
            }
            issues.push(Summary {
                number: value.get("number").and_then(Value::as_u64).unwrap_or(0),
                state: text(&value, "state").to_uppercase(),
                title: text(&value, "title"),
                author: login(&value),
                body: text(&value, "body"),
            });
            if issues.len() == LIMIT {
                return Ok(issues);
            }
        }
        next = page.next;
    }
    Ok(issues)
}

/// Picker rows aligned like `column -t`: number, state, title, author.
fn labels(issues: &[Summary]) -> Vec<String> {
    let width = |f: &dyn Fn(&Summary) -> usize| issues.iter().map(f).max().unwrap_or(0);
    let number = width(&|i| i.number.to_string().len());
    let state = width(&|i| i.state.chars().count());
    let title = width(&|i| i.title.chars().count());
    issues
        .iter()
        .map(|i| {
            format!(
                "{:<number$}  {:<state$}  {:<title$}  (@{})",
                i.number, i.state, i.title, i.author
            )
        })
        .collect()
}

/// The issue preview, as `lib/preview.sh --issue`.
fn preview(issue: &Summary) -> String {
    let body = if issue.body.is_empty() {
        "*No description provided.*"
    } else {
        &issue.body
    };
    format!("# {}\n\n{body}", issue.title)
}

/// Renders one issue and its whole comment thread.
fn render(client: &Client, number: u64) -> Result<String, ApiError> {
    let slug = client.repo.slug();
    let issue = client.get_json(&format!("/repos/{slug}/issues/{number}"))?;
    let comments = client.get_all(&format!(
        "/repos/{slug}/issues/{number}/comments?per_page=100"
    ))?;

    let title = issue
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let mut parts = vec![
        format!("# Issue #{number}: {title}"),
        format!("URL: {}\n", text(&issue, "html_url")),
        "## Description".to_owned(),
    ];
    let body = text(&issue, "body");
    parts.push(if body.is_empty() {
        "*No description provided.*".to_owned()
    } else {
        body
    });
    if !comments.is_empty() {
        parts.push("\n## Discussion Thread".to_owned());
        for (index, comment) in comments.iter().enumerate() {
            parts.push(format!("\n### Comment {} (@{})", index + 1, login(comment)));
            parts.push(text(comment, "body"));
        }
    }
    Ok(parts.join("\n"))
}

/// Fails early with a specific message when there is no terminal to pick on.
fn require_terminal() -> io::Result<()> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map(drop)
        .map_err(|error| {
            crate::output::fail(
                format!("selecting issues needs an interactive terminal ({error})"),
                "run 'focal issues' from a terminal",
            )
        })
}

pub fn run(args: Args) -> io::Result<()> {
    let client = Client::for_current_repo()?;
    require_terminal()?;
    eprintln!("info: Fetching issues...");
    let issues = list(&client, args.all)?;
    if issues.is_empty() {
        let which = if args.all { "" } else { "open " };
        eprintln!("info: No {which}issues found in {}.", client.repo.slug());
        return Ok(());
    }

    let items = labels(&issues)
        .into_iter()
        .zip(&issues)
        .map(|(label, issue)| Item {
            search: label.clone(),
            label,
            weight: issue.body.len() as u64,
        })
        .collect();
    let previews: Arc<Vec<String>> = Arc::new(issues.iter().map(preview).collect());
    let loader: Arc<dyn Fn(usize) -> String + Send + Sync> =
        Arc::new(move |id| previews[id].clone());
    let options = Options {
        prompt: "Select issues: ".into(),
        header: vec!["TAB: multi-select | CTRL-A: select all | ENTER: confirm".into()],
        multi: true,
    };
    let ids = match picker::pick(items, &options, Some(loader))? {
        Outcome::Selected(ids) if !ids.is_empty() => ids,
        // Legacy exits quietly when fzf is cancelled or selects nothing.
        _ => return Ok(()),
    };

    let numbers: Vec<u64> = ids.iter().map(|&id| issues[id].number).collect();
    let sections: Vec<String> = numbers
        .iter()
        .map(|&number| {
            render(&client, number).unwrap_or_else(|error| {
                eprintln!(
                    "error: could not fetch issue #{number}: {}\n  hint: {}",
                    error.message, error.hint
                );
                format!("# Issue #{number}\n\n[Could not fetch issue details or comments]")
            })
        })
        .collect();

    let message = match numbers.as_slice() {
        [number] => format!("Copied context for Issue #{number}"),
        _ => format!("Copied context for {} issues", numbers.len()),
    };
    let mut payload = header()?;
    payload.push_str(&sections.join("\n\n---\n\n"));
    deliver(payload, &message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(number: u64, title: &str, body: &str) -> Summary {
        Summary {
            number,
            state: "OPEN".into(),
            title: title.into(),
            author: "octocat".into(),
            body: body.into(),
        }
    }

    #[test]
    fn aligns_labels_like_column() {
        let rows = labels(&[summary(7, "Short", ""), summary(120, "A longer title", "")]);
        assert_eq!(
            rows,
            [
                "7    OPEN  Short           (@octocat)",
                "120  OPEN  A longer title  (@octocat)",
            ]
        );
    }

    #[test]
    fn previews_title_and_body() {
        assert_eq!(preview(&summary(1, "T", "Body")), "# T\n\nBody");
        assert_eq!(
            preview(&summary(1, "T", "")),
            "# T\n\n*No description provided.*"
        );
    }
}
