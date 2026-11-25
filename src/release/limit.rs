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

/// `text` cut to about `limit` characters at a line boundary, with any fence left open by
/// the cut closed and `notice` appended. Text within the limit is returned unchanged.
fn clip(text: &str, limit: usize, notice: &str) -> String {
    if chars(text) <= limit {
        return text.to_owned();
    }
    let mut out = before_last_line(prefix(text, limit.saturating_sub(100))).to_owned();
    if let Some(fence) = open_fence(&out) {
        out.push('\n');
        out.push_str(&fence);
    }
    out.push_str(notice);
    out
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
    let max_chars = max_tokens.saturating_mul(4);
    if chars(content) <= max_chars {
        return content.to_owned();
    }
    let marker = "\n## Completeness Notes\n";
    let (mut body, separator, notes) = match content.rfind(marker) {
        Some(at) => (
            content[..at].to_owned(),
            true,
            content[at + marker.len()..].to_owned(),
        ),
        None => (content.to_owned(), false, String::new()),
    };
    // Keep the aggregate impact even when detailed PRs consume the budget. The real section
    // follows every PR description, so the last match is the right one.
    let impact_start = body.rfind("\n## Release Impact\n");
    let mut impact = String::new();
    if let Some(start) = impact_start {
        let end = body[start + 1..]
            .find("\n## ")
            .map_or(body.len(), |at| start + 1 + at);
        impact = body[start..end].to_owned();
        if chars(&impact) > max_chars / 5 {
            let lines: Vec<&str> = impact.lines().collect();
            // Never more than the lines before the two-line tail (summary and closing
            // fence), so the tail is not repeated.
            let keep = lines.len().saturating_sub(2).min(40);
            let tail = &lines[lines.len().saturating_sub(2).max(keep)..];
            impact = format!(
                "{}\n...[diffstat paths abbreviated]\n{}\n",
                lines[..keep].join("\n"),
                tail.join("\n")
            );
            if let Some(fence) = open_fence(&impact) {
                impact.push_str(&fence);
                impact.push('\n');
            }
            impact = clip(
                &impact,
                max_chars / 5,
                "\n...[release impact abbreviated]\n",
            );
        }
        body = format!("{}{}", &body[..start], &body[end..]);
    }
    let mut warning = format!(
        "\n\n## Output Limit\n\n* WARNING: output exceeded the approximate {}-token budget. Some release detail is omitted; this payload is incomplete. Rerun with a narrower range or increase --max-tokens; 0 disables this limit.\n",
        thousands(max_tokens)
    );
    // The notes keep their own share of the footer budget, so a long impact section can
    // never crowd them out, and large numbers of diagnostics cannot defeat the bound.
    let notes_limit = (max_chars / 3)
        .saturating_sub(chars(&impact))
        .max(max_chars / 10);
    let notes = if separator {
        clip(
            &format!("{marker}{notes}"),
            notes_limit,
            "\n* Additional diagnostic detail omitted by output limit.\n",
        )
    } else {
        String::new()
    };
    let footer = format!("{impact}{notes}");
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

    fn report(impact_lines: usize, width: usize, notes: usize) -> String {
        let detail = "x".repeat(4_000);
        let stat: String = (0..impact_lines)
            .map(|n| format!(" path/{n:03}/{} | 2 +-\n", "p".repeat(width)))
            .collect();
        let notes: String = (0..notes).map(|n| format!("* note {n}\n")).collect();
        format!(
            "# Release Context\n\n## Pull Requests\n\n{detail}\n\n## Release Impact\n\n```text\n{stat} {impact_lines} files changed\n```\n\n## Completeness Notes\n\n{notes}"
        )
    }

    #[test]
    fn limited_output_keeps_fences_balanced_and_notes_present() {
        // A long impact (many lines) and a short but wide impact (few, long lines).
        for (lines, width) in [(60, 80), (12, 400), (35, 120)] {
            let out = limit(&report(lines, width, 5), 1_000);
            assert_eq!(
                open_fence(&out),
                None,
                "{lines} lines of width {width}:\n{out}"
            );
            assert!(out.contains("## Release Impact"), "{lines}x{width}");
            assert!(
                out.contains("## Completeness Notes\n\n* note 0"),
                "{lines}x{width}"
            );
            assert!(out.chars().count() <= 4_000 + 400, "{lines}x{width}");
        }
    }

    #[test]
    fn heading_inside_a_pr_description_is_not_moved() {
        let content = format!(
            "# Release Context\n\n## Pull Requests\n\nBody quoting\n## Release Impact\n\nfrom a PR\n{}\n\n## Release Impact\n\nreal stat\n\n## Completeness Notes\n\n* done\n",
            "y".repeat(8_000)
        );
        let out = limit(&content, 500);
        let footer = out.split("## Output Limit").nth(1).unwrap();
        assert!(footer.contains("real stat"));
        assert!(!footer.contains("from a PR"));
    }

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
