//! Release-note instructions, previous-release style, pinned media evidence, and the PR
//! shape shared by GraphQL and REST association results.

use serde_json::Value;
use std::fmt::Write as _;

const MEDIA_EXTENSIONS: [&str; 11] = [
    "apng", "avif", "gif", "jpeg", "jpg", "mov", "mp4", "png", "svg", "webm", "webp",
];

const BOT_AUTHORS: [&str; 6] = [
    "dependabot[bot]",
    "dependabot",
    "renovate[bot]",
    "renovate",
    "github-actions[bot]",
    "github-actions",
];

/// Whether an author label matches a known dependency or CI bot.
pub fn is_bot_author(label: &str) -> bool {
    BOT_AUTHORS.iter().any(|bot| {
        label.contains(&format!("(@{bot})")) || label.contains(&format!("@{bot}")) || label == *bot
    })
}

/// The drafting request, using the actual target commit.
pub fn prompt(version: &str, head: &str) -> String {
    format!(
        "# Release Notes Request

Write a title and Markdown release description for my GitHub release `{version}`, targeting commit `{head}`.
Use the previous release title and description included below as a reference for how I usually structure release notes, including tone, headings, and level of detail. Use it for style only: describe only changes supported by this release's evidence. If no previous release is available, use a concise, readable structure and tell me the style reference was unavailable. If the version is `vX.Y.Z`, keep that placeholder and ask me to confirm the version; do not invent a version.

If repository assets such as demos, screenshots, GIFs, or SVGs would help explain the release, include relevant links or Markdown embeds. Use the asset inventory below or resolve assets yourself at the release commit. Every repository asset URL must be pinned to the full release commit `{head}`; never use a branch, HEAD, or a movable tag, and do not reuse unpinned links from the previous release. Prefer a raw URL for image embeds and a file URL for video links. Do not assume an asset is useful from its filename alone; inspect it if you can.

If you cannot resolve or verify an asset, use a clearly labeled placeholder such as `[Demo](<ASSET_URL: path at {head}>)`. Do not invent URLs or claim a link was verified without checking it. The inventory confirms paths in the local Git tree, not remote availability or rendering. Include a separate checklist after the draft listing every proposed asset link or placeholder, its path and pinned commit, and what I should verify. Keep that checklist outside the release description so I can review it before publishing.

Return the proposed release title, the release description ready to paste into GitHub, and any verification items or gaps in the supplied evidence.
"
    )
}

/// Percent-encodes a path as Python's `urllib.parse.quote(path, safe="/")`.
pub fn quote_path(path: &str) -> String {
    let mut out = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"_.-~/".contains(&byte) {
            out.push(byte as char);
        } else {
            write!(out, "%{byte:02X}").unwrap();
        }
    }
    out
}

/// Whether a path's final suffix (as `PurePosixPath.suffix`) is an image or video type.
fn is_media(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => {
            MEDIA_EXTENSIONS.contains(&name[dot + 1..].to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

/// Tracked media at the immutable head with encoded, commit-pinned URLs.
pub fn assets(files: &[String], head: &str, repo_url: Option<&str>) -> String {
    let mut rows = Vec::new();
    for path in files.iter().filter(|path| is_media(path)) {
        match repo_url {
            Some(url) => {
                let prefix = url.trim_end_matches('/');
                let encoded = quote_path(path);
                rows.push(format!(
                    "* `{path}` — [file]({prefix}/blob/{head}/{encoded}), [raw]({prefix}/raw/{head}/{encoded})"
                ));
            }
            None => rows.push(format!(
                "* `{path}` — `<ASSET_URL: {path} at {head}>` (repository URL unavailable)"
            )),
        }
    }
    if rows.is_empty() {
        rows.push("*No tracked image/video assets found at the release commit. Use a placeholder if a relevant demo needs to be supplied.*".into());
    }
    format!(
        "Tracked image/video candidates at `{head}`. Paths exist in the local Git tree; remote URLs and rendering have not been verified. Choose only assets relevant to this release.\n\n{}",
        rows.join("\n")
    )
}

/// A published release rendered as the style reference.
pub fn reference(release: &Value, tag: &str) -> String {
    let text = |key: &str| {
        release
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    format!(
        "**Tag:** {}\n**Title:** {}\n**URL:** {}\n\n**Description (style reference only):**\n\n{}",
        text("tag_name").unwrap_or(tag),
        text("name").unwrap_or(tag),
        text("html_url").unwrap_or(""),
        text("body").unwrap_or("*No description provided.*")
    )
}

/// A merged or open pull request associated with a commit.
#[derive(Clone)]
pub struct Pr {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub merged: bool,
    pub merge_commit: Option<String>,
    pub login: String,
    pub is_bot_account: bool,
    pub labels: Vec<String>,
}

impl Pr {
    pub fn is_bot(&self) -> bool {
        is_bot_author(&self.login) || self.is_bot_account
    }

    fn build(value: &Value, url_key: &str, merged_key: &str, user_key: &str) -> Option<Self> {
        let text =
            |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let user = value.get(user_key).filter(|user| user.is_object());
        let labels = value
            .get("labels")
            .map(|labels| labels.get("nodes").unwrap_or(labels));
        Some(Self {
            number: value.get("number")?.as_u64()?,
            title: text(value, "title").unwrap_or_else(|| "Untitled".into()),
            body: text(value, "body").unwrap_or_default(),
            url: text(value, url_key).unwrap_or_default(),
            merged: value
                .get(merged_key)
                .is_some_and(|merged| !merged.is_null() && merged != ""),
            merge_commit: None,
            login: user
                .and_then(|user| text(user, "login"))
                .unwrap_or_else(|| "Unknown".into()),
            is_bot_account: user.is_some_and(|user| {
                user.get("type").or_else(|| user.get("__typename")) == Some(&Value::from("Bot"))
            }),
            labels: labels
                .and_then(Value::as_array)
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(|label| text(label, "name"))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// From a GraphQL `associatedPullRequests` node.
    pub fn from_graphql(node: &Value) -> Option<Self> {
        let mut pr = Self::build(node, "url", "mergedAt", "author")?;
        pr.merge_commit = node
            .pointer("/mergeCommit/oid")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(pr)
    }

    /// From a REST `commits/{sha}/pulls` item.
    pub fn from_rest(item: &Value) -> Option<Self> {
        let mut pr = Self::build(item, "html_url", "merged_at", "user")?;
        pr.merge_commit = item
            .get("merge_commit_sha")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(pr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognizes_bot_authors() {
        for label in [
            "dependabot[bot]",
            "renovate",
            "Bump x (@dependabot)",
            "@github-actions",
        ] {
            assert!(is_bot_author(label), "{label}");
        }
        for label in ["alice", "dependabot-fan", "Renovate"] {
            assert!(!is_bot_author(label), "{label}");
        }
    }

    #[test]
    fn quotes_paths_like_python() {
        assert_eq!(
            quote_path("docs/demo space#1.SVG"),
            "docs/demo%20space%231.SVG"
        );
        assert_eq!(quote_path("a/ü~_.-b"), "a/%C3%BC~_.-b");
    }

    #[test]
    fn media_detection_uses_the_final_suffix() {
        assert!(is_media("docs/demo.SVG"));
        assert!(is_media("a.tar.png"));
        assert!(is_media(".hidden.gif"));
        assert!(!is_media(".png"));
        assert!(!is_media("png"));
        assert!(!is_media("demo.svg.txt"));
    }

    #[test]
    fn normalizes_both_association_shapes() {
        let graphql = Pr::from_graphql(&json!({
            "number": 7, "title": "Fix", "body": null, "url": "u", "mergedAt": "2026-01-01",
            "mergeCommit": {"oid": "abc"}, "author": {"login": "dependabot", "__typename": "Bot"},
            "labels": {"nodes": [{"name": "deps"}]}
        }))
        .unwrap();
        assert!(graphql.merged && graphql.is_bot());
        assert_eq!(graphql.merge_commit.as_deref(), Some("abc"));
        assert_eq!(graphql.labels, ["deps"]);
        let rest = Pr::from_rest(&json!({
            "number": 8, "title": "Open", "html_url": "h", "merged_at": null, "user": null,
            "labels": [{"name": "bug"}]
        }))
        .unwrap();
        assert!(!rest.merged);
        assert_eq!(rest.login, "Unknown");
        assert_eq!(rest.labels, ["bug"]);
    }
}
