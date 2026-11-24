"""`--repo`: cached checkouts of remote repositories.

Local bare repositories stand in for remotes, and XDG_CACHE_HOME points at a temporary
cache, so nothing touches the network or the user's cache.
"""

import fcntl
import hashlib
import os
import subprocess
import time
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
CLI = Path(os.environ.get("FOCAL_TEST_BIN", ROOT / "bin/focal")).resolve()
LEGACY = "FOCAL_TEST_BIN" not in os.environ
DAY = 86400


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=a", "-c", "user.email=a@example.com", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
        env={**os.environ, "GIT_CONFIG_NOSYSTEM": "1"},
    ).stdout.strip()


class Remote:
    """A bare repository on `main` plus a working clone used to push commits."""

    def __init__(self, tmp_path: Path):
        self.tmp = tmp_path
        self.bare = tmp_path / "remote.git"
        git(tmp_path, "init", "-q", "--bare", "-b", "main", str(self.bare))
        git(self.bare, "config", "uploadpack.allowFilter", "true")
        self.work = tmp_path / "upstream"
        git(tmp_path, "clone", "-q", str(self.bare), str(self.work))
        git(self.work, "checkout", "-q", "-b", "main")
        self.commit("README.md", "remote v1\n")
        self.url = f"file://{self.bare}"

    def commit(self, name: str, text: str, branch: str = "main") -> None:
        (self.work / name).write_text(text)
        git(self.work, "add", "-A")
        git(self.work, "commit", "-qm", f"update {name}")
        git(self.work, "push", "-q", "origin", f"HEAD:{branch}")


@pytest.fixture
def remote(tmp_path):
    return Remote(tmp_path)


def cache_root(tmp_path: Path) -> Path:
    return tmp_path / "cache" / "focal" / "repos"


def entry_path(tmp_path: Path, url: str) -> Path:
    digest = hashlib.sha256(url.encode()).hexdigest()[:16]
    base = url.rstrip("/").rsplit("/", 1)[-1].removesuffix(".git")
    return cache_root(tmp_path) / f"{base}_{digest}"


def env(tmp_path: Path, **extra: str) -> dict[str, str]:
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    return {
        "PATH": "" if os.environ.get("FOCAL_TEST_EMPTY_PATH") else os.environ["PATH"],
        "HOME": str(home),
        "XDG_CACHE_HOME": str(tmp_path / "cache"),
        "LC_ALL": "C.UTF-8",
        "NO_COLOR": "1",
        "GIT_CONFIG_NOSYSTEM": "1",
        "FOCAL_OMIT_WORKSPACE": "1",
        **extra,
    }


def focal(tmp_path: Path, *args: str, **extra: str):
    cwd = tmp_path / "elsewhere"
    cwd.mkdir(exist_ok=True)
    result = subprocess.run(
        [CLI, *args],
        cwd=cwd,
        env=env(tmp_path, **extra),
        stdin=subprocess.DEVNULL,
        capture_output=True,
        check=False,
    )
    return result.returncode, result.stdout.decode(), result.stderr.decode()


def readme(tmp_path: Path, url: str, *before: str, **extra: str):
    return focal(tmp_path, *before, "--repo", url, "files", "README.md", **extra)


def age(path: Path, days: int) -> None:
    stamp = time.time() - days * DAY
    os.utime(path, (stamp, stamp))


def scratch(tmp_path: Path) -> list[str]:
    return sorted(
        p.name
        for p in cache_root(tmp_path).iterdir()
        if p.name.startswith((".tmp-", ".trash-"))
    )


def test_cold_clone_runs_the_command_in_the_checkout(tmp_path, remote):
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "info: Cloning remote repository into cache...\n" in err
    assert "remote v1" in out
    entry = entry_path(tmp_path, remote.url)
    assert (entry / ".git" / ".focal_created").is_file()
    assert (entry / ".git" / ".focal_accessed").is_file()


def test_warm_update_picks_up_new_commits(tmp_path, remote):
    assert readme(tmp_path, remote.url)[0] == 0
    marker = entry_path(tmp_path, remote.url) / ".git" / ".focal_accessed"
    age(marker, 2)
    remote.commit("README.md", "remote v2\n")
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "info: Updating cached remote repository...\n" in err
    assert "Cloning" not in err
    assert "remote v2" in out
    assert time.time() - marker.stat().st_mtime < DAY


def test_local_edits_in_the_cache_are_discarded(tmp_path, remote):
    assert readme(tmp_path, remote.url)[0] == 0
    (entry_path(tmp_path, remote.url) / "README.md").write_text("tampered\n")
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "remote v1" in out
    assert "tampered" not in out


@pytest.mark.parametrize(
    "args",
    [
        ("--repo", "{url}", "files", "README.md"),
        ("--repo={url}", "files", "README.md"),
        ("files", "--repo", "{url}", "README.md"),
        ("files", "README.md", "--repo={url}"),
    ],
)
def test_repo_flag_on_either_side_of_the_command(tmp_path, remote, args):
    code, out, err = focal(tmp_path, *(a.format(url=remote.url) for a in args))
    assert code == 0, err
    assert "remote v1" in out


def test_plain_path_remote(tmp_path, remote):
    code, out, err = readme(tmp_path, str(remote.bare))
    assert code == 0, err
    assert "remote v1" in out
    assert entry_path(tmp_path, str(remote.bare)).is_dir()


@pytest.mark.skipif(LEGACY, reason="legacy runs two git clones into the same path")
def test_simultaneous_cold_invocations_share_one_clone(tmp_path, remote):
    for i in range(200):
        remote.work.joinpath(f"bulk{i}.txt").write_text(os.urandom(4096).hex())
    remote.commit("README.md", "remote v1\n")
    cwd = tmp_path / "elsewhere"
    cwd.mkdir()
    procs = [
        subprocess.Popen(
            [CLI, "--repo", remote.url, "files", "README.md"],
            cwd=cwd,
            env=env(tmp_path),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        for _ in range(2)
    ]
    results = [p.communicate(timeout=60) for p in procs]
    for proc, (out, err) in zip(procs, results, strict=True):
        assert proc.returncode == 0, err.decode()
        assert b"remote v1" in out
    errs = b"".join(err for _, err in results).decode()
    assert errs.count("Cloning remote repository into cache") == 1
    entries = [p for p in cache_root(tmp_path).iterdir() if not p.name.startswith(".")]
    assert entries == [entry_path(tmp_path, remote.url)]
    assert scratch(tmp_path) == []


def test_leftover_scratch_clone_is_discarded(tmp_path, remote):
    root = cache_root(tmp_path)
    name = entry_path(tmp_path, remote.url).name
    leftover = root / f".tmp-99999-{name}"
    (leftover / ".git").mkdir(parents=True)
    (leftover / "half-written").write_text("x")
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "info: Cloning remote repository into cache...\n" in err
    assert "remote v1" in out
    if not LEGACY:
        assert scratch(tmp_path) == []


@pytest.mark.skipif(LEGACY, reason="legacy clones directly into the entry")
def test_killed_clone_is_retried_cleanly(tmp_path, remote):
    for i in range(400):
        remote.work.joinpath(f"bulk{i}.txt").write_text(os.urandom(16384).hex())
    remote.commit("README.md", "remote v1\n")
    cwd = tmp_path / "elsewhere"
    cwd.mkdir()
    proc = subprocess.Popen(
        [CLI, "--repo", remote.url, "files", "README.md"],
        cwd=cwd,
        env=env(tmp_path),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    root = cache_root(tmp_path)
    deadline = time.monotonic() + 20
    while proc.poll() is None and time.monotonic() < deadline:
        if root.is_dir() and any(p.name.startswith(".tmp-") for p in root.iterdir()):
            proc.kill()
            break
        time.sleep(0.001)
    proc.wait()
    entry = entry_path(tmp_path, remote.url)
    # Either the clone finished first or the kill left no half-made entry.
    assert proc.returncode == 0 or not entry.exists()
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "remote v1" in out
    assert scratch(tmp_path) == []


@pytest.mark.xfail(LEGACY, reason="legacy keeps using a damaged entry", strict=True)
def test_damaged_entry_is_cloned_again(tmp_path, remote):
    # What a killed `git clone` into the entry path leaves behind.
    entry = entry_path(tmp_path, remote.url)
    (entry / ".git").mkdir(parents=True)
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "remote v1" in out
    assert scratch(tmp_path) == []


def test_default_branch_rename_is_followed(tmp_path, remote):
    assert readme(tmp_path, remote.url)[0] == 0
    git(remote.work, "branch", "-q", "-m", "main", "trunk")
    remote.commit("README.md", "remote on trunk\n", branch="trunk")
    git(remote.bare, "symbolic-ref", "HEAD", "refs/heads/trunk")
    git(remote.bare, "branch", "-q", "-D", "main")
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "remote on trunk" in out


@pytest.mark.xfail(LEGACY, reason="legacy keeps the old local branch name", strict=True)
def test_default_branch_rename_switches_the_local_branch(tmp_path, remote):
    assert readme(tmp_path, remote.url)[0] == 0
    git(remote.work, "branch", "-q", "-m", "main", "trunk")
    remote.commit("README.md", "remote on trunk\n", branch="trunk")
    git(remote.bare, "symbolic-ref", "HEAD", "refs/heads/trunk")
    code, out, err = readme(tmp_path, remote.url, FOCAL_OMIT_WORKSPACE="")
    assert code == 0, err
    assert "Git Branch: trunk\n" in out


def test_offline_warm_cache_is_reused(tmp_path, remote):
    assert readme(tmp_path, remote.url)[0] == 0
    remote.bare.rename(tmp_path / "moved.git")
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert (
        "warning: Could not update the cached repository; using the cached copy.\n"
        in err
    )
    assert "remote v1" in out


def test_unreachable_cold_remote_fails_without_an_entry(tmp_path):
    url = f"file://{tmp_path}/missing.git"
    code, out, err = readme(tmp_path, url)
    assert code != 0
    assert out == ""
    assert not entry_path(tmp_path, url).exists()
    if not LEGACY:
        assert "error: could not clone the remote repository" in err
        assert scratch(tmp_path) == []


def legacy_clone(tmp_path: Path, remote: Remote, *flags: str) -> Path:
    entry = entry_path(tmp_path, remote.url)
    entry.parent.mkdir(parents=True, exist_ok=True)
    git(tmp_path, "clone", "-q", *flags, remote.url, str(entry))
    (entry / ".git" / "legacy-sentinel").write_text("kept")
    return entry


def test_legacy_full_entry_is_updated_in_place(tmp_path, remote):
    entry = legacy_clone(tmp_path, remote)
    remote.commit("README.md", "remote v2\n")
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "info: Updating cached remote repository...\n" in err
    assert "remote v2" in out
    assert (entry / ".git" / "legacy-sentinel").is_file()
    assert (entry / ".git" / ".focal_accessed").is_file()


def test_legacy_partial_entry_is_updated(tmp_path, remote):
    entry = legacy_clone(tmp_path, remote, "--filter=blob:none")
    remote.commit("README.md", "remote v2\n")
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "info: Updating cached remote repository...\n" in err
    assert "remote v2" in out
    if not LEGACY:
        # Replaced by a full clone, since libgit2 cannot fetch missing blobs.
        config = (entry / ".git" / "config").read_text()
        assert "partialclone" not in config
        assert "promisor" not in config


def test_legacy_partial_entry_is_reused_offline(tmp_path, remote):
    legacy_clone(tmp_path, remote, "--filter=blob:none")
    remote.bare.rename(tmp_path / "moved.git")
    code, out, err = readme(tmp_path, remote.url, FOCAL_OMIT_WORKSPACE="")
    assert code == 0, err
    assert (
        "warning: Could not update the cached repository; using the cached copy.\n"
        in err
    )
    assert "remote v1" in out
    assert "Git Branch: main\n" in out


def make_entry(root: Path, name: str, days: int, marker: bool = True) -> Path:
    entry = root / name
    (entry / ".git").mkdir(parents=True)
    if marker:
        (entry / ".git" / ".focal_created").touch()
        (entry / ".git" / ".focal_accessed").touch()
        age(entry / ".git" / ".focal_accessed", days)
    age(entry, days)
    return entry


def test_cleanup_prunes_stale_entries_but_not_the_active_one(tmp_path, remote):
    root = cache_root(tmp_path)
    assert readme(tmp_path, remote.url)[0] == 0
    active = entry_path(tmp_path, remote.url)
    age(active / ".git" / ".focal_accessed", 40)
    stale = make_entry(root, "old_0123456789abcdef", 40)
    unmarked = make_entry(root, "older_0123456789abcdef", 40, marker=False)
    fresh = make_entry(root, "fresh_0123456789abcdef", 3)
    fresh_unmarked = make_entry(root, "cloning_0123456789abcdef", 0, marker=False)
    (root / ".last_cleanup").unlink()
    code, out, err = readme(tmp_path, remote.url)
    assert code == 0, err
    assert "remote v1" in out
    assert not stale.exists()
    assert not unmarked.exists()
    assert active.is_dir()
    assert fresh.is_dir()
    assert fresh_unmarked.is_dir()
    assert "info: Pruning stale cached repository: old_0123456789abcdef\n" in err
    assert "info: Pruning legacy cached repository: older_0123456789abcdef\n" in err


def test_cleanup_honors_the_ttl_and_runs_once_a_day(tmp_path, remote):
    root = cache_root(tmp_path)
    assert readme(tmp_path, remote.url)[0] == 0
    entry = make_entry(root, "old_0123456789abcdef", 10)
    assert readme(tmp_path, remote.url, FOCAL_REPO_TTL_DAYS="5")[0] == 0
    assert entry.is_dir()  # throttled by .last_cleanup
    (root / ".last_cleanup").unlink()
    assert readme(tmp_path, remote.url, FOCAL_REPO_TTL_DAYS="5")[0] == 0
    assert not entry.exists()


@pytest.mark.xfail(LEGACY, reason="legacy prunes any old directory", strict=True)
def test_cleanup_spares_directories_focal_does_not_own(tmp_path, remote):
    root = cache_root(tmp_path)
    assert readme(tmp_path, remote.url)[0] == 0
    notes = root / "notes"
    notes.mkdir()
    (notes / "keep.txt").write_text("mine")
    age(notes, 40)
    (root / ".last_cleanup").unlink()
    assert readme(tmp_path, remote.url)[0] == 0
    assert (notes / "keep.txt").is_file()


@pytest.mark.xfail(LEGACY, reason="legacy takes no locks", strict=True)
def test_cleanup_spares_an_entry_in_use(tmp_path, remote):
    root = cache_root(tmp_path)
    assert readme(tmp_path, remote.url)[0] == 0
    busy = make_entry(root, "busy_0123456789abcdef", 40)
    (root / ".locks").mkdir(exist_ok=True)
    (root / ".last_cleanup").unlink()
    with open(root / ".locks" / "busy_0123456789abcdef.use", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_SH)
        assert readme(tmp_path, remote.url)[0] == 0
    assert busy.is_dir()


@pytest.mark.skipif(LEGACY, reason="legacy takes no locks")
def test_entry_stays_locked_while_the_command_runs(tmp_path, remote, terminal):
    assert readme(tmp_path, remote.url)[0] == 0
    name = entry_path(tmp_path, remote.url).name
    lock_path = cache_root(tmp_path) / ".locks" / f"{name}.use"
    cwd = tmp_path / "elsewhere"
    session = terminal(
        cwd,
        "--repo",
        remote.url,
        "files",
        env={"XDG_CACHE_HOME": str(tmp_path / "cache"), "GIT_CONFIG_NOSYSTEM": "1"},
    )
    session.wait_for("Files: ")
    with open(lock_path, "w") as lock, pytest.raises(BlockingIOError):
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    session.send(b"\x1b")
    code, err, _, _ = session.finish()
    assert code == 0, err
    with open(lock_path, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
