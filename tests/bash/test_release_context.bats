#!/usr/bin/env bats

setup() {
    RELEASE_CONTEXT_BIN="${BATS_TEST_DIRNAME}/../../libexec/release-context"
    ORIG_DIR="$PWD"
}

teardown() {
    cd "$ORIG_DIR"
    rm -rf "$BATS_TEST_TMPDIR/repo" "$BATS_TEST_TMPDIR/bin"
}

@test "release-context -h prints usage and options" {
    run "$RELEASE_CONTEXT_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal release-context [level] [options]"* ]]
    [[ "$output" == *"--minor, -m"* ]]
    [[ "$output" == *"--major"* ]]
    [[ "$output" == *"--patch"* ]]
    [[ "$output" == *"--base"* ]]
    [[ "$output" == *"--head"* ]]
}

@test "release-context rejects invalid arguments" {
    run "$RELEASE_CONTEXT_BIN" --unknown-flag

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: unknown argument: --unknown-flag"* ]]
}

@test "release-context rejects invalid --level argument" {
    run "$RELEASE_CONTEXT_BIN" --level invalid

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: invalid release level 'invalid'"* ]]
}

make_release_repo() {
    rm -rf "$BATS_TEST_TMPDIR/repo" "$BATS_TEST_TMPDIR/bin"
    mkdir -p "$BATS_TEST_TMPDIR/repo" "$BATS_TEST_TMPDIR/bin"
    cat > "$BATS_TEST_TMPDIR/bin/gh" <<'MOCK'
#!/usr/bin/env bash
case "$1" in
  repo) echo '{"nameWithOwner":"org/repo","url":"https://github.com/org/repo"}' ;;
  release) echo '{"tagName":"v1.0.0","name":"v1.0.0","body":"Release notes","url":"https://github.com/org/repo/releases/tag/v1.0.0"}' ;;
  api) echo '[]' ;;
  *) exit 1 ;;
esac
MOCK
    chmod +x "$BATS_TEST_TMPDIR/bin/gh"
    export PATH="$BATS_TEST_TMPDIR/bin:$PATH"
    cd "$BATS_TEST_TMPDIR/repo"
    git init -q -b main
    git config user.name Alice
    git config user.email alice@example.com
    echo 'Initial background' > README.md
    git add README.md
    git commit -qm 'Initial commit'
    git tag v1.0.0
    echo 'Release change' > app.txt
    git add app.txt
    git commit -qm 'Release change'
}

@test "release-context validates missing option values" {
    for flag in --base --head --bots --path --version --previous-release; do
        run "$RELEASE_CONTEXT_BIN" "$flag"
        [ "$status" -eq 1 ]
        [[ "$output" == *"$flag requires an argument"* ]]
    done
}

@test "release-context outputs a compact range without checkout background" {
    make_release_repo
    run "$RELEASE_CONTEXT_BIN"
    [ "$status" -eq 0 ]
    [[ "$output" == *"<context>"* ]]
    [[ "$output" == *"# Release Context: v1.0.0 to HEAD"* ]]
    [[ "$output" == *"Release change"* ]]
    [[ "$output" == *"## Release Impact"* ]]
    [[ "$output" == *"## Completeness Notes"* ]]
    [[ "$output" != *"# Repository Tree"* ]]
    [[ "$output" != *"# Workspace Context"* ]]
    [[ "$output" != *"Initial background"* ]]
}

@test "release-context optionally appends clearly labeled project background" {
    make_release_repo
    run "$RELEASE_CONTEXT_BIN" --project-context
    [ "$status" -eq 0 ]
    [[ "$output" == *"# Project Background (current checkout, not the release snapshot)"* ]]
    [[ "$output" == *"# Repository Tree"* ]]
    [[ "$output" == *"Initial background"* ]]
    [[ "$output" == *"first 500 lines"* ]]
}

@test "release-context forwards historical endpoints and focused diff" {
    make_release_repo
    historical=$(git rev-parse HEAD)
    echo 'Later change' > later.txt
    git add later.txt
    git commit -qm 'Later change'
    run "$RELEASE_CONTEXT_BIN" --base v1.0.0 --head "$historical" --diff --path app.txt
    [ "$status" -eq 0 ]
    [[ "$output" == *"## Focused Diff"* ]]
    [[ "$output" == *"+Release change"* ]]
    [[ "$output" != *"Later change"* ]]
    [[ "$output" != *"later.txt"* ]]
}

@test "release-context forwards bot policy" {
    make_release_repo
    echo 'Dependency change' > deps.txt
    git add deps.txt
    git commit -qm 'Dependency change' --author 'dependabot[bot] <bot@example.com>'
    run "$RELEASE_CONTEXT_BIN" --bots summary
    [ "$status" -eq 0 ]
    [[ "$output" == *"Bot Updates (compact summary)"* ]]
    run "$RELEASE_CONTEXT_BIN" --bots exclude
    [ "$status" -eq 0 ]
    [[ "$output" == *"1 commits omitted from detail"* ]]
    [[ "$output" != *"Dependency change"* ]]
    [[ "$output" == *"deps.txt"* ]]
}

@test "release-context requires diff for path filters" {
    make_release_repo
    run "$RELEASE_CONTEXT_BIN" --path app.txt
    [ "$status" -eq 1 ]
    [[ "$output" == *"--path requires --diff"* ]]
}

@test "release-context validates bot policy and refs" {
    make_release_repo
    run "$RELEASE_CONTEXT_BIN" --bots invalid
    [ "$status" -eq 1 ]
    [[ "$output" == *"Invalid value for '--bots'"* ]]
    run "$RELEASE_CONTEXT_BIN" --head does-not-exist
    [ "$status" -eq 1 ]
    [[ "$output" == *"Git command failed"* ]]
}

@test "release-context includes a versioned drafting prompt and explicit style reference" {
    make_release_repo
    cat > "$BATS_TEST_TMPDIR/bin/gh" <<'MOCK'
#!/usr/bin/env bash
case "$1" in
  repo) echo '{"nameWithOwner":"org/repo","url":"https://github.com/org/repo"}' ;;
  release) echo '{"tagName":"v1.0.0","name":"Previous release title","body":"## My usual structure","url":"https://github.com/org/repo/releases/tag/v1.0.0"}' ;;
  api) echo '[]' ;;
  *) exit 1 ;;
esac
MOCK
    run "$RELEASE_CONTEXT_BIN" --version v1.1.0 --previous-release v1.0.0
    [ "$status" -eq 0 ]
    [[ "$output" == *'my GitHub release `v1.1.0`'* ]]
    [[ "$output" == *"Previous release title"* ]]
    [[ "$output" == *"## My usual structure"* ]]
    [[ "$output" == *"pinned to the full release commit"* ]]
    [[ "$output" == *"separate checklist after the draft"* ]]
}

@test "release-context version targets an existing tag before later HEAD commits" {
    make_release_repo
    git tag v1.1.0
    echo 'Later work' > later.txt
    git add later.txt
    git commit -qm 'Later work'
    run "$RELEASE_CONTEXT_BIN" --version v1.1.0 --previous-release v1.0.0
    [ "$status" -eq 0 ]
    [[ "$output" == *"# Release Context: v1.0.0 to v1.1.0"* ]]
    [[ "$output" == *"Release change"* ]]
    [[ "$output" != *"Later work"* ]]
    [[ "$output" != *"later.txt"* ]]
    run "$RELEASE_CONTEXT_BIN" --version v1.1.0 --head HEAD
    [ "$status" -eq 0 ]
    [[ "$output" == *"# Release Context: v1.1.0 to HEAD"* ]]
    [[ "$output" == *"Later work"* ]]
}
