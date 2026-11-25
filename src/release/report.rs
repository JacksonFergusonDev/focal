//! Builds release evidence from Git membership and commit-associated GitHub PRs.
//!
//! Git determines membership; GitHub supplies optional PR metadata. Unknown or failed
//! associations remain visible as unmatched commits and completeness notes.

use super::collect::{BATCH_SIZE, Done, Fetch, GitHub, Job, Lookup};
use super::limit::{general, prefix, thousands};
use super::notes::{self, Pr, is_bot_author};
use super::range::{self, Commit, Endpoints, History, Tag};
use super::{Bots, Options};
use crate::file::fenced;
use crate::github::{self, Client};
use crate::{diff, git, output};
use git2::Repository;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Component, Path};
use std::sync::LazyLock;

static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());

fn git_error(error: git2::Error) -> io::Error {
    io::Error::other(format!("Git operation failed: {}", error.message()))
}

fn unknown_revision(revision: &str) -> io::Error {
    output::fail(
        format!("unknown revision or not a commit: '{revision}'"),
        "check the ref name, or fetch it first: git fetch --tags",
    )
}

/// Resolves a cwd-relative pathspec against the repository root, collapsing `.` and `..`.
fn root_spec(prefix: &Path, arg: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in prefix.join(arg).components() {
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    let mut spec = parts.join("/");
    if arg.ends_with('/') && !spec.is_empty() {
        spec.push('/');
    }
    spec
}

/// A GitHub client for the checkout, or the reason enrichment is unavailable.
fn client() -> Result<Client, String> {
    let reason = |error: io::Error| {
        let text = error.to_string();
        match text.split_once("\n  hint: ") {
            Some((message, hint)) => format!("GitHub enrichment unavailable: {message} ({hint})"),
            None => format!("GitHub enrichment unavailable: {text}"),
        }
    };
    let repo = github::current_repo().map_err(reason)?;
    let token = github::token(&repo.host).map_err(reason)?;
    Ok(Client::new(repo, token, github::DEFAULT_TIMEOUT))
}

/// The previous published stable release before the target, chosen locally from a
/// release listing, plus the notes that explain the choice.
fn choose_reference(
    repo: &Repository,
    history: &History,
    tags: &[Tag],
    releases: &[Value],
    version: &str,
) -> Result<(Option<String>, Vec<String>), git2::Error> {
    let by_name: HashMap<&str, &Tag> = tags.iter().map(|tag| (tag.name.as_str(), tag)).collect();
    let mut candidates = Vec::new();
    let mut missing = 0;
    for release in releases {
        let tag_name = release.get("tag_name").and_then(Value::as_str);
        let flag = |key: &str| release.get(key).and_then(Value::as_bool) == Some(true);
        if flag("draft") || flag("prerelease") || tag_name == Some(version) {
            continue;
        }
        let Some(tag) = tag_name
            .and_then(|name| by_name.get(name))
            .filter(|tag| history.contains(tag.commit))
        else {
            missing += 1;
            continue;
        };
        if tag.commit != history.head {
            candidates.push(*tag);
        }
    }
    let mut notes = Vec::new();
    if missing > 0 {
        notes.push(format!(
            "{missing} published release tags were not reachable in the local checkout; they were excluded from automatic style selection."
        ));
    }
    let chosen = history.describe(repo, &candidates, candidates.len().max(1))?;
    if chosen.is_some() {
        notes.push("Previous release style reference is the nearest locally reachable published stable release before the target commit, independent of the comparison base.".into());
    }
    Ok((chosen.map(|tag| tag.name.clone()), notes))
}

/// GitHub enrichment: the repository URL, the style reference, and associations.
struct Enrichment {
    repo_url: Option<String>,
    repo_known: bool,
    reference: String,
    reference_notes: Vec<String>,
    lookups: Vec<Lookup>,
    notes: Vec<String>,
}

fn enrich(
    repo: &Repository,
    github: &GitHub,
    history: &History,
    tags: &[Tag],
    commits: &[Commit],
    version: &str,
    previous: Option<&str>,
) -> Result<Enrichment, git2::Error> {
    let mut notes = Vec::new();
    let skipped = || commits.iter().map(|_| Lookup::Skipped).collect();
    let lookup = match github.slug() {
        Some(slug) => match github.request(&format!("repos/{slug}"), None, &mut notes) {
            Fetch::Ok(value) => Some(value),
            _ => None,
        },
        None => None,
    };
    let Some(info) = lookup else {
        return Ok(Enrichment {
            repo_url: None,
            repo_known: false,
            reference: "*Previous release unavailable: GitHub repository lookup failed.*".into(),
            reference_notes: vec![
                "WARNING: previous release style reference could not be fetched.".into(),
            ],
            lookups: skipped(),
            notes,
        });
    };
    let text = |key: &str| info.get(key).and_then(Value::as_str).map(str::to_owned);
    let slug = text("full_name")
        .or_else(|| github.slug())
        .unwrap_or_default();
    let repo_url = text("html_url");

    // First wave: the release listing (or the explicit reference) and every GraphQL batch.
    let shas: Vec<String> = commits.iter().map(|commit| commit.sha.clone()).collect();
    let mut jobs = vec![match previous {
        Some(tag) => Job::View(tag.to_owned()),
        None => Job::Releases,
    }];
    for (index, batch) in shas.chunks(BATCH_SIZE).enumerate() {
        jobs.push(Job::Batch {
            offset: index * BATCH_SIZE,
            total: shas.len(),
            shas: batch.to_vec(),
        });
    }
    let mut first = github.run(&slug, jobs).into_iter();
    let (reference_done, reference_job_notes) = first.next().expect("reference job");
    notes.extend(reference_job_notes);
    let mut lookups: Vec<Option<Lookup>> = Vec::new();
    for (done, job_notes) in first {
        notes.extend(job_notes);
        if let Done::Batch(results) = done {
            lookups.extend(results);
        }
    }

    // Second wave: the chosen reference and REST listings for overflowing associations.
    let mut reference_notes = Vec::new();
    let mut reference = None;
    let mut view = None;
    match (previous, reference_done) {
        (Some(tag), Done::View(fetch)) => {
            reference_notes.push(format!(
                "Previous release style reference explicitly selected with --previous-release: {tag}; this does not change the Git range."
            ));
            view = Some((tag.to_owned(), Some(fetch)));
        }
        (_, Done::Releases(Ok(releases))) => {
            let (chosen, choice_notes) = choose_reference(repo, history, tags, &releases, version)?;
            reference_notes.extend(choice_notes);
            match chosen {
                Some(tag) => view = Some((tag, None)),
                None => {
                    reference = Some(
                        "*No previous published stable release reachable before the target commit was found.*"
                            .to_owned(),
                    );
                }
            }
        }
        _ => {
            reference = Some("*Previous release unavailable: release listing failed.*".into());
            reference_notes =
                vec!["WARNING: previous release discovery failed; style reference omitted.".into()];
        }
    }
    let mut jobs = Vec::new();
    if let Some((tag, None)) = &view {
        jobs.push(Job::View(tag.clone()));
    }
    let rest: Vec<usize> = (0..lookups.len())
        .filter(|&index| lookups[index].is_none())
        .collect();
    jobs.extend(rest.iter().map(|&index| Job::Rest(shas[index].clone())));
    let mut second = github.run(&slug, jobs).into_iter();
    if let Some((tag, fetch)) = &mut view {
        if fetch.is_none() {
            let (done, job_notes) = second.next().expect("view job");
            notes.extend(job_notes);
            if let Done::View(result) = done {
                *fetch = Some(result);
            }
        }
        reference = Some(match fetch.take() {
            Some(Fetch::Ok(release)) if release.is_object() => notes::reference(&release, tag),
            _ => {
                reference_notes.push(format!(
                    "WARNING: could not fetch previous release {tag}; style reference omitted."
                ));
                format!("*Previous release `{tag}` unavailable: release lookup failed.*")
            }
        });
    }
    for (index, (done, job_notes)) in rest.into_iter().zip(second) {
        notes.extend(job_notes);
        if let Done::Rest(result) = done {
            lookups[index] = Some(result);
        }
    }
    Ok(Enrichment {
        repo_url,
        repo_known: true,
        reference: reference.unwrap_or_default(),
        reference_notes,
        lookups: lookups
            .into_iter()
            .map(|lookup| lookup.unwrap_or(Lookup::Skipped))
            .collect(),
        notes,
    })
}

/// Collects an exact Git range, associated PR intent, and release impact.
pub fn render(repo: &Repository, cwd: &Path, options: &Options) -> io::Result<String> {
    let explicit_head = options.head.is_some();
    let mut head_ref = options.head.clone().unwrap_or_else(|| "HEAD".into());
    let mut range_head_ref = head_ref.clone();
    if let Some(version) = options.version.as_deref().filter(|_| !explicit_head) {
        let tag = format!("refs/tags/{version}");
        if range::commit(repo, &tag).is_some() {
            head_ref = version.to_owned();
            range_head_ref = tag;
            eprintln!("info: Using existing version tag {version} as the release target");
        } else {
            eprintln!("info: Version tag {version} is not available locally; drafting from HEAD");
        }
    }
    let head = range::commit(repo, &range_head_ref).ok_or_else(|| unknown_revision(&head_ref))?;
    let tags = range::tags(repo).map_err(git_error)?;
    let history = History::new(repo, head).map_err(git_error)?;
    let Endpoints { base, head, label } = match &options.base {
        Some(base_ref) => Endpoints {
            base: Some(range::commit(repo, base_ref).ok_or_else(|| unknown_revision(base_ref))?),
            head,
            label: base_ref.clone(),
        },
        None => range::select_base(repo, &history, &tags, &options.level).map_err(git_error)?,
    };
    let version = match &options.version {
        Some(version) => version.clone(),
        None => {
            let at_head: Vec<&str> = tags
                .iter()
                .filter(|tag| tag.commit == head)
                .map(|tag| tag.name.as_str())
                .collect();
            if at_head.contains(&head_ref.as_str()) {
                head_ref.clone()
            } else {
                at_head
                    .into_iter()
                    .find(|tag| range::is_semver(tag))
                    .unwrap_or("vX.Y.Z")
                    .to_owned()
            }
        }
    };
    let commits = range::commits(repo, base, head).map_err(git_error)?;
    let head_sha = head.to_string();

    let mut parts = vec![
        notes::prompt(&version, &head_sha),
        format!("# Release Context: {label} to {head_ref}"),
        format!(
            "**Base:** {label}{}",
            match base {
                Some(base) => format!(" (`{base}`, exclusive)"),
                None => " (all reachable history, including root commits)".into(),
            }
        ),
        format!("**Head:** `{head_sha}` (inclusive)"),
        format!("**Commits in range:** {}", commits.len()),
    ];
    let mut notes = vec![
        "Membership is determined by Git ancestry. PR associations are enrichment; PR descriptions may have changed since the release.".to_owned(),
        "Only merged PRs returned for in-range commits are included; no default-branch or merge-date filter is applied.".to_owned(),
    ];
    if let Some(base) = base
        && !range::is_ancestor(repo, base, head).map_err(git_error)?
    {
        notes.push("Base is not an ancestor of head: this is Git set difference, not a linear release interval.".into());
    }
    if repo.is_shallow() {
        notes.push("WARNING: shallow checkout; available Git history and tag selection may be incomplete. Fetch full history and tags.".into());
    }

    eprintln!(
        "info: Collecting release context for {} commits ({label} to {head_ref})",
        commits.len()
    );
    let github = match client() {
        Ok(client) => GitHub::new(
            Some(client),
            options.request_timeout,
            options.github_timeout,
        ),
        Err(reason) => {
            let github = GitHub::new(None, options.request_timeout, options.github_timeout);
            github.stop(&reason);
            github
        }
    };
    let enrichment = enrich(
        repo,
        &github,
        &history,
        &tags,
        &commits,
        &version,
        options.previous_release.as_deref(),
    )
    .map_err(git_error)?;
    if !enrichment.repo_known {
        notes.push("WARNING: GitHub repository lookup failed; all commits are shown without PR enrichment.".into());
    }
    notes.extend(enrichment.reference_notes.iter().cloned());
    parts.push("\n## Previous Release (style reference)\n".into());
    parts.push(enrichment.reference.clone());
    parts.push("\n## Repository Assets (pinned to release commit)\n".into());
    let files = range::tree_files(repo, head).map_err(git_error)?;
    parts.push(notes::assets(
        &files,
        &head_sha,
        enrichment.repo_url.as_deref(),
    ));

    // Group commits under merged PRs in first-commit order.
    let mut order: Vec<u64> = Vec::new();
    let mut groups: HashMap<u64, (Pr, Vec<&Commit>)> = HashMap::new();
    let mut unmatched: Vec<&Commit> = Vec::new();
    let (mut failed, mut skipped) = (Vec::new(), 0);
    let mut multiple = 0;
    for (commit, lookup) in commits.iter().zip(&enrichment.lookups) {
        let prs: &[Pr] = match lookup {
            Lookup::Found(prs) => prs,
            Lookup::Failed => {
                failed.push(commit.sha.as_str());
                &[]
            }
            Lookup::Skipped => {
                failed.push(commit.sha.as_str());
                skipped += 1;
                &[]
            }
        };
        let mut merged: Vec<&Pr> = prs.iter().filter(|pr| pr.merged).collect();
        merged.sort_by_key(|pr| pr.number);
        if merged.is_empty() {
            unmatched.push(commit);
            continue;
        }
        // Prefer the actual merge/squash commit match, then a stable PR number.
        let pr = merged
            .iter()
            .find(|pr| pr.merge_commit.as_deref() == Some(commit.sha.as_str()))
            .unwrap_or(&merged[0]);
        if merged.len() > 1 {
            multiple += 1;
            if multiple <= 20 {
                let numbers: Vec<String> =
                    merged.iter().map(|pr| format!("#{}", pr.number)).collect();
                notes.push(format!(
                    "Commit `{}` has multiple merged PR associations: {}; grouped under #{}.",
                    &commit.sha[..12],
                    numbers.join(", "),
                    pr.number
                ));
            }
        }
        let entry = groups.entry(pr.number).or_insert_with(|| {
            order.push(pr.number);
            ((*pr).clone(), Vec::new())
        });
        // The latest association wins, as with the legacy dictionary update.
        entry.0 = (*pr).clone();
        entry.1.push(commit);
    }

    eprintln!("info: Formatting local commit paths and release impact");
    let bots = options.bots;
    let limited = options.max_tokens > 0;
    let (mut bot_groups, mut bot_commits) = (0, 0);
    let mut bot_rows = Vec::new();
    if limited {
        parts.push("\n## PR Index (all associated PRs)\n".into());
        for number in &order {
            let (pr, entries) = &groups[number];
            if bots != Bots::Exclude || !pr.is_bot() {
                parts.push(format!(
                    "* [#{number}: {}]({}) — {} in-range commits",
                    pr.title,
                    pr.url,
                    entries.len()
                ));
            }
        }
        notes.push("Bounded output includes a PR index subject to the overall size limit; detailed descriptions, path lists, and commit lists may be abbreviated. Use --max-tokens 0 for full detail.".into());
    }
    parts.push("\n## Pull Requests\n".into());
    let detail = |count: usize| if limited { count.min(5) } else { count };
    for number in &order {
        let (pr, entries) = &groups[number];
        let changed: Vec<&str> = entries
            .iter()
            .flat_map(|commit| commit.paths.iter().map(String::as_str))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if pr.is_bot() {
            bot_groups += 1;
            bot_commits += entries.len();
            bot_rows.push(format!(
                "* PR #{number}: {} — {}; {} commits, {} paths",
                pr.title,
                pr.url,
                entries.len(),
                changed.len()
            ));
            if bots != Bots::Include {
                continue;
            }
        }
        let mut body = COMMENT.replace_all(&pr.body, "").trim().to_owned();
        let length = body.chars().count();
        if limited && length > 800 {
            let excerpt = prefix(&body, 800);
            let cut = match excerpt.rfind('\n') {
                Some(at) if excerpt[..at].chars().count() > 400 => &excerpt[..at],
                _ => excerpt.rsplit_once(' ').map_or(excerpt, |(head, _)| head),
            };
            body = format!(
                "{cut}\n\n*[Description excerpt; {} characters in full description.]*",
                thousands(length)
            );
        }
        parts.push(format!("### PR #{number}: {}", pr.title));
        parts.push(format!("**Author:** @{}", pr.login));
        parts.push(format!("**URL:** {}", pr.url));
        if !pr.labels.is_empty() {
            parts.push(format!("**Labels:** {}", pr.labels.join(", ")));
        }
        parts.push("\n**Intent / Description:**".into());
        parts.push(if body.is_empty() {
            "*No description provided.*".into()
        } else {
            body
        });
        parts.push("\n**In-range commits:**".into());
        for commit in &entries[..detail(entries.len())] {
            parts.push(format!(
                "* `{}` — {} ({})",
                &commit.sha[..12],
                commit.subject,
                commit.author
            ));
        }
        if limited && entries.len() > 5 {
            parts.push(format!(
                "* {} additional in-range commits omitted from detail.",
                entries.len() - 5
            ));
        }
        parts.push("\n**Changed paths (in-range commits):**".into());
        for path in &changed[..detail(changed.len())] {
            parts.push(format!("* `{path}`"));
        }
        if limited && changed.len() > 5 {
            parts.push(format!(
                "* {} additional changed paths omitted from detail.",
                changed.len() - 5
            ));
        }
    }
    if groups.is_empty() {
        parts.push("*No merged PR associations found.*".into());
    } else if groups.len() == bot_groups && bots != Bots::Include {
        parts.push("*All associated PRs are covered by the bot policy below.*".into());
    }

    parts.push("\n## Unmatched Commits\n".into());
    let mut visible = 0;
    for commit in &unmatched {
        let row = format!(
            "* `{}` — {} ({})",
            &commit.sha[..12],
            commit.subject,
            commit.author
        );
        if is_bot_author(&commit.author) {
            bot_commits += 1;
            bot_rows.push(row.clone());
            if bots != Bots::Include {
                continue;
            }
        }
        visible += 1;
        parts.push(row);
        for path in &commit.paths[..detail(commit.paths.len())] {
            parts.push(format!("  * `{path}`"));
        }
        if limited && commit.paths.len() > 5 {
            parts.push(format!(
                "  * {} additional changed paths omitted from detail.",
                commit.paths.len() - 5
            ));
        }
    }
    if visible == 0 {
        parts.push("*None under the selected bot policy.*".into());
    }
    if bots == Bots::Summary && !bot_rows.is_empty() {
        parts.push("\n## Bot Updates (compact summary)\n".into());
        parts.extend(bot_rows);
    }

    let impact = range::net_diff(repo, base, head, &[]).map_err(git_error)?;
    let stat = if impact.deltas().len() == 0 {
        String::new()
    } else {
        diff::stat(&impact).map_err(git_error)?.trim().to_owned()
    };
    parts.push("\n## Release Impact\n".into());
    parts.push("Net changes between the resolved endpoints (including bot updates):".into());
    parts.push(fenced(
        if stat.is_empty() {
            "No net file changes."
        } else {
            &stat
        },
        "text",
    ));
    if options.diff {
        let prefix = git::prefix(repo, cwd);
        let specs: Vec<String> = options
            .paths
            .iter()
            .map(|path| root_spec(&prefix, path))
            .collect();
        let focused = range::net_diff(repo, base, head, &specs).map_err(git_error)?;
        let patch = diff::patch(&focused).map_err(git_error)?;
        let patch = patch.trim_end_matches('\n');
        parts.push("\n## Focused Diff\n".into());
        parts.push(format!(
            "Paths: {}",
            if options.paths.is_empty() {
                "all paths".to_owned()
            } else {
                options.paths.join(", ")
            }
        ));
        parts.push(fenced(
            if patch.is_empty() {
                "No matching changes."
            } else {
                patch
            },
            "diff",
        ));
    }

    notes.push(format!(
        "Collected {} commits; {} merged PRs; {} unmatched commits; {} unresolved association lookups; {multiple} ambiguous associations.",
        commits.len(),
        groups.len(),
        unmatched.len(),
        failed.len()
    ));
    notes.push(format!(
        "Bot policy: {}; {bot_groups} bot PRs and {bot_commits} commits {}",
        bots.name(),
        match bots {
            Bots::Include => "included in full.",
            Bots::Summary => "summarized.",
            Bots::Exclude => "omitted from detail (still included in release impact).",
        }
    ));
    notes.push("PR association pages are fetched until exhausted; no PR count ceiling. Output size limits are reported explicitly when detail is omitted.".into());
    notes.push("Unmatched commits may be direct pushes, cherry-picks, or commits without a GitHub association; no PR attribution is inferred from commit subjects.".into());
    notes.push("Tag selection uses locally available, reachable stable vMAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH tags; fetch tags before collecting a release.".into());
    if multiple > 20 {
        notes.push(format!(
            "Ambiguity details limited to 20 commits; {} additional ambiguous commits are counted above.",
            multiple - 20
        ));
    }
    if !failed.is_empty() {
        let listed: Vec<String> = failed
            .iter()
            .take(20)
            .map(|sha| format!("`{}`", &sha[..12]))
            .collect();
        let more = if failed.len() > 20 {
            format!(" (and {} more)", failed.len() - 20)
        } else {
            String::new()
        };
        notes.push(format!(
            "WARNING: PR enrichment is incomplete for: {}{more}",
            listed.join(", ")
        ));
        notes.push(format!(
            "Unresolved association lookups: {} failed, {skipped} skipped because GitHub collection was unavailable or stopped.",
            failed.len() - skipped
        ));
    }
    notes.extend(enrichment.notes);
    notes.extend(github.stop_note());
    notes.push(format!(
        "GitHub collection made {} requests; per-request timeout {}s, total GitHub budget {}s. Unknown associations include failed or skipped lookups.",
        github.requests(),
        general(options.request_timeout),
        general(options.github_timeout)
    ));
    parts.push("\n## Completeness Notes\n".into());
    parts.extend(notes.iter().map(|note| format!("* {note}")));
    Ok(parts.join("\n") + "\n")
}
