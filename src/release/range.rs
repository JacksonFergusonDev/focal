//! Release membership from local Git: tag selection, endpoints, commits, and paths.

use git2::{Oid, Repository, Sort, Tree};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

static SEMVER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$").unwrap());

/// Whether a tag names a stable `vMAJOR.MINOR.PATCH` or `MAJOR.MINOR.PATCH` release.
pub fn is_semver(tag: &str) -> bool {
    SEMVER.is_match(tag)
}

/// Whether a stable tag qualifies as a base for `level` (minor: x.y.0, major: x.0.0).
fn fits_level(tag: &str, level: &str) -> bool {
    let Some(parts) = SEMVER.captures(tag) else {
        return false;
    };
    let (minor, patch) = (&parts[2], &parts[3]);
    match level {
        "minor" => patch == "0",
        "major" => minor == "0" && patch == "0",
        _ => true,
    }
}

pub struct Tag {
    pub name: String,
    pub commit: Oid,
    annotated: bool,
    time: i64,
}

/// Local tags that peel to a commit, in refname order.
pub fn tags(repo: &Repository) -> Result<Vec<Tag>, git2::Error> {
    let mut tags = Vec::new();
    for reference in repo.references_glob("refs/tags/*")? {
        let reference = reference?;
        let Some(name) = reference
            .name()
            .and_then(|name| name.strip_prefix("refs/tags/"))
        else {
            continue;
        };
        let Ok(commit) = reference.peel_to_commit() else {
            continue;
        };
        let tag = reference.peel_to_tag().ok();
        let annotated = tag.is_some();
        let time = tag
            .and_then(|tag| tag.tagger().map(|tagger| tagger.when().seconds()))
            .unwrap_or(0);
        tags.push(Tag {
            name: name.to_owned(),
            commit: commit.id(),
            annotated,
            time,
        });
    }
    tags.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    Ok(tags)
}

/// The commits reachable from a head, in commit-date order, which is the order
/// `git describe` discovers candidate tags in.
pub struct History {
    pub head: Oid,
    order: Vec<Oid>,
    reachable: HashSet<Oid>,
}

impl History {
    pub fn new(repo: &Repository, head: Oid) -> Result<Self, git2::Error> {
        let mut walk = repo.revwalk()?;
        walk.set_sorting(Sort::TIME)?;
        walk.push(head)?;
        let order = walk.collect::<Result<Vec<_>, _>>()?;
        let reachable = order.iter().copied().collect();
        Ok(Self {
            head,
            order,
            reachable,
        })
    }

    /// Whether a tag is merged into the head, as in `git tag --merged <head>`.
    pub fn contains(&self, commit: Oid) -> bool {
        self.reachable.contains(&commit)
    }

    /// The nearest of `candidates`, as `git describe --tags --abbrev=0 --match …` picks it:
    /// the fewest commits between the tag and the head, with ties going to the tag found
    /// first in commit-date order. Annotated tags win over lightweight ones on one commit.
    pub fn describe<'t>(
        &self,
        repo: &Repository,
        candidates: &[&'t Tag],
        max_candidates: usize,
    ) -> Result<Option<&'t Tag>, git2::Error> {
        let mut best: HashMap<Oid, &Tag> = HashMap::new();
        for &tag in candidates {
            let keep = best.get(&tag.commit).is_some_and(|current| {
                (current.annotated && !tag.annotated)
                    || (current.annotated == tag.annotated
                        && (!tag.annotated || current.time >= tag.time))
            });
            if !keep {
                best.insert(tag.commit, tag);
            }
        }
        let mut found = Vec::new();
        for oid in &self.order {
            if let Some(tag) = best.get(oid) {
                found.push(*tag);
                if found.len() >= max_candidates {
                    break;
                }
            }
        }
        let mut choice: Option<(usize, &Tag)> = None;
        for tag in found {
            let (depth, _) = repo.graph_ahead_behind(self.head, tag.commit)?;
            if choice.is_none_or(|(best, _)| depth < best) {
                choice = Some((depth, tag));
            }
        }
        Ok(choice.map(|(_, tag)| tag))
    }
}

/// Resolves a revision to a commit, as `git rev-parse --verify <rev>^{commit}`.
pub fn commit(repo: &Repository, revision: &str) -> Option<Oid> {
    repo.revparse_single(&format!("{revision}^{{commit}}"))
        .ok()
        .and_then(|object| object.peel_to_commit().ok())
        .map(|commit| commit.id())
}

pub struct Endpoints {
    pub base: Option<Oid>,
    pub head: Oid,
    pub label: String,
}

/// Selects the nearest reachable stable SemVer tag for `level` before `head`, or the
/// repository beginning when there is none.
pub fn select_base(
    repo: &Repository,
    history: &History,
    tags: &[Tag],
    level: &str,
) -> Result<Endpoints, git2::Error> {
    // A tag on the target describes that release, so compare with its predecessor.
    let candidates: Vec<&Tag> = tags
        .iter()
        .filter(|tag| fits_level(&tag.name, level))
        .filter(|tag| history.contains(tag.commit) && tag.commit != history.head)
        .collect();
    // `git describe` considers ten candidates by default.
    Ok(match history.describe(repo, &candidates, 10)? {
        Some(tag) => Endpoints {
            base: Some(tag.commit),
            head: history.head,
            label: tag.name.clone(),
        },
        None => Endpoints {
            base: None,
            head: history.head,
            label: "Repository beginning".into(),
        },
    })
}

pub struct Commit {
    pub sha: String,
    pub author: String,
    pub subject: String,
    /// Paths changed against the first parent (or the empty tree for a root commit).
    pub paths: Vec<String>,
}

fn tree_paths(
    repo: &Repository,
    old: Option<&Tree>,
    new: &Tree,
) -> Result<Vec<String>, git2::Error> {
    let mut diff = repo.diff_tree_to_tree(old, Some(new), None)?;
    crate::diff::renames(&mut diff)?;
    let mut paths = Vec::new();
    for delta in diff.deltas() {
        if let Some(path) = crate::diff::delta_path(&delta) {
            paths.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(paths)
}

/// Commits in `base..head` (or everything reachable, including root commits), oldest
/// first in topological order, as `git log --reverse --topo-order`.
pub fn commits(
    repo: &Repository,
    base: Option<Oid>,
    head: Oid,
) -> Result<Vec<Commit>, git2::Error> {
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    walk.push(head)?;
    if let Some(base) = base {
        walk.hide(base)?;
    }
    let mut commits = Vec::new();
    for oid in walk {
        let commit = repo.find_commit(oid?)?;
        let parent = match commit.parent_count() {
            0 => None,
            _ => Some(commit.parent(0)?.tree()?),
        };
        let paths = tree_paths(repo, parent.as_ref(), &commit.tree()?)?;
        commits.push(Commit {
            sha: commit.id().to_string(),
            author: String::from_utf8_lossy(commit.author().name_bytes()).into_owned(),
            subject: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default())
                .into_owned(),
            paths,
        });
    }
    Ok(commits)
}

/// Whether `base` is an ancestor of (or equal to) `head`.
pub fn is_ancestor(repo: &Repository, base: Oid, head: Oid) -> Result<bool, git2::Error> {
    Ok(base == head || repo.graph_descendant_of(head, base)?)
}

/// The net diff between the endpoints; an empty-tree base includes every root commit.
pub fn net_diff<'r>(
    repo: &'r Repository,
    base: Option<Oid>,
    head: Oid,
    pathspecs: &[String],
) -> Result<git2::Diff<'r>, git2::Error> {
    let old = base.map(|oid| repo.find_commit(oid)?.tree()).transpose()?;
    let new = repo.find_commit(head)?.tree()?;
    let mut options = crate::diff::options();
    for spec in pathspecs {
        options.pathspec(spec);
    }
    let mut diff = repo.diff_tree_to_tree(old.as_ref(), Some(&new), Some(&mut options))?;
    crate::diff::renames(&mut diff)?;
    Ok(diff)
}

/// Regular files in the head tree (no symlinks or submodules), with their paths.
pub fn tree_files(repo: &Repository, head: Oid) -> Result<Vec<String>, git2::Error> {
    let tree = repo.find_commit(head)?.tree()?;
    let mut files = Vec::new();
    tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
        // Modes 100644 and 100755; symlinks (120000) and submodules (160000) are skipped.
        let regular = entry.filemode() & 0o170000 == 0o100000;
        if regular {
            let name = String::from_utf8_lossy(entry.name_bytes());
            files.push(format!("{root}{name}"));
        }
        git2::TreeWalkResult::Ok
    })?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_semver_and_levels() {
        for tag in ["v1.2.3", "1.0.0", "v0.10.0"] {
            assert!(is_semver(tag), "{tag}");
        }
        for tag in ["v1.2", "v01.2.3", "v1.2.3-rc.1", "release-1.0.0", "V1.2.3"] {
            assert!(!is_semver(tag), "{tag}");
        }
        assert!(fits_level("v1.2.3", "patch"));
        assert!(!fits_level("v1.2.3", "minor"));
        assert!(fits_level("v1.2.0", "minor"));
        assert!(!fits_level("v1.2.0", "major"));
        assert!(fits_level("v2.0.0", "major"));
    }
}
