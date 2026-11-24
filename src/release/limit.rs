//! Explicit size limits for release-context payloads, and small text helpers.
//!
//! Lengths are counted in characters, as the legacy Python limiter does.

use regex::Regex;
use std::sync::LazyLock;

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(`{3,}|~{3,})(.*)$").unwrap());

/// `1234567` as `1,234,567`.
pub fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A float as Python's `:g` renders it for the timeouts used here (six significant digits,
/// no trailing zeros).
pub fn general(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return format!("{value}");
    }
    let magnitude = value.abs().log10().floor() as i32;
    if !(-4..6).contains(&magnitude) {
        let text = format!("{value:.5e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        let exponent: i32 = exponent.parse().unwrap_or(0);
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exponent.abs());
    }
    let decimals = (5 - magnitude).max(0) as usize;
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

/// The first `count` characters of `text`.
pub fn prefix(text: &str, count: usize) -> &str {
    let end = text
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(i, _)| i);
    &text[..end]
}

/// Everything before the last newline, or all of `text` without one.
fn before_last_line(text: &str) -> &str {
    text.rsplit_once('\n').map_or(text, |(head, _)| head)
}

/// The fence left open at the end of `text`, honoring nested longer fences.
pub fn open_fence(text: &str) -> Option<String> {
    let mut open: Option<String> = None;
    for line in text.lines() {
        let Some(parts) = FENCE.captures(line) else {
            continue;
        };
        let (marker, rest) = (&parts[1], &parts[2]);
        match &open {
            None => open = Some(marker.to_owned()),
            Some(current)
                if marker.as_bytes()[0] == current.as_bytes()[0]
                    && marker.len() >= current.len()
                    && rest.trim().is_empty() =>
            {
                open = None;
            }
            Some(_) => {}
        }
    }
    open
}

/// Bounds approximate tokens while retaining endpoint metadata, the release impact, and
/// the final counts. `0` disables the limit.
pub fn limit(content: &str, max_tokens: usize) -> String {
    if max_tokens == 0 {
        return content.to_owned();
    }
    let max_chars = max_tokens * 4;
    if chars(content) <= max_chars {
        return content.to_owned();
    }
    let marker = "\n## Completeness Notes\n";
    let (mut body, separator, mut notes) = match content.rfind(marker) {
        Some(at) => (
            content[..at].to_owned(),
            true,
            content[at + marker.len()..].to_owned(),
        ),
        None => (content.to_owned(), false, String::new()),
    };
    // Keep the aggregate impact even when detailed PRs consume the budget.
    let impact_start = body.find("\n## Release Impact\n");
    if let Some(start) = impact_start {
        let end = body[start + 1..]
            .find("\n## ")
            .map_or(body.len(), |at| start + 1 + at);
        let mut impact = body[start..end].to_owned();
        if chars(&impact) > max_chars / 5 {
            let lines: Vec<&str> = impact.lines().collect();
            let tail = &lines[lines.len().saturating_sub(2)..];
            impact = format!(
                "{}\n...[diffstat paths abbreviated]\n{}\n",
                lines[..lines.len().min(40)].join("\n"),
                tail.join("\n")
            );
        }
        body = format!("{}{}", &body[..start], &body[end..]);
        notes = format!("{impact}{marker}{notes}");
    }
    let mut warning = format!(
        "\n\n## Output Limit\n\n* WARNING: output exceeded the approximate {}-token budget. Some release detail is omitted; this payload is incomplete. Rerun with a narrower range or increase --max-tokens; 0 disables this limit.\n",
        thousands(max_tokens)
    );
    let mut footer = if impact_start.is_some() {
        notes
    } else if separator {
        format!("{marker}{notes}")
    } else {
        String::new()
    };
    // Large numbers of diagnostics must not defeat the output bound either.
    let footer_limit = max_chars / 3;
    if chars(&footer) > footer_limit {
        footer = format!(
            "{}\n* Additional diagnostic detail omitted by output limit.\n",
            before_last_line(prefix(&footer, footer_limit.saturating_sub(100)))
        );
    }
    let allowance = max_chars.saturating_sub(chars(&warning) + chars(&footer) + 150);
    let mut clipped = before_last_line(prefix(&body, allowance)).to_owned();
    // Close an open fenced code block when a requested diff or description is cut.
    if let Some(fence) = open_fence(&clipped) {
        clipped.push('\n');
        clipped.push_str(&fence);
    }
    let omitted = chars(&body).saturating_sub(chars(&clipped));
    warning.push_str(&format!(
        "* {} characters of release detail omitted from the end of the payload.\n",
        thousands(omitted)
    ));
    eprintln!(
        "warning: Release context exceeded ~{} tokens; detail omitted. Use --max-tokens 0 for full output.",
        thousands(max_tokens)
    );
    clipped + &warning + &footer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers_like_python() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(24000), "24,000");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(general(10.0), "10");
        assert_eq!(general(60.0), "60");
        assert_eq!(general(0.5), "0.5");
        assert_eq!(general(1.25), "1.25");
        assert_eq!(general(0.123456789), "0.123457");
        assert_eq!(general(1e-5), "1e-05");
        assert_eq!(general(1234567.0), "1.23457e+06");
    }

    #[test]
    fn detects_open_fences() {
        assert_eq!(open_fence("```diff\n+a\n"), Some("```".into()));
        assert_eq!(open_fence("```diff\n+a\n```\n"), None);
        assert_eq!(open_fence("````md\n```\ninner\n```\n"), Some("````".into()));
        assert_eq!(open_fence("~~~\n```\n"), Some("~~~".into()));
        assert_eq!(open_fence("```\n``` not a close\n"), Some("```".into()));
    }

    #[test]
    fn small_content_is_unchanged() {
        let text = "# Release\n\n## Completeness Notes\n\n* ok\n";
        assert_eq!(limit(text, 1000), text);
        assert_eq!(limit(&"x".repeat(10_000), 0).len(), 10_000);
    }

    #[test]
    fn keeps_impact_and_notes_and_closes_fences() {
        let detail = format!("```diff\n{}", "+line\n".repeat(3000));
        let content = format!(
            "# Release Context\n\n## Pull Requests\n\n{detail}\n```\n\n## Release Impact\n\nstat\n\n## Completeness Notes\n\n* Collected 3 commits\n"
        );
        let limited = limit(&content, 1000);
        assert!(limited.chars().count() <= 4000);
        assert!(limited.starts_with("# Release Context"));
        assert!(limited.contains("## Output Limit"));
        assert!(limited.contains("characters of release detail omitted"));
        let (before, after) = limited.split_once("## Output Limit").unwrap();
        assert!(open_fence(before).is_none());
        assert!(after.contains("## Release Impact\n\nstat"));
        assert!(after.ends_with("* Collected 3 commits\n"));
    }

    #[test]
    fn long_footers_are_bounded() {
        let notes: String = (0..2000).map(|i| format!("* note {i}\n")).collect();
        let content = format!("# Head\n\n## Completeness Notes\n\n{notes}");
        let limited = limit(&content, 1000);
        assert!(limited.chars().count() <= 4000);
        assert!(limited.contains("Additional diagnostic detail omitted by output limit."));
    }
}
