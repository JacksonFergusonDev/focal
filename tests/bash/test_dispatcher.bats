#!/usr/bin/env bats

setup() {
    ORIG_DIR="$PWD"
    FOCAL_BIN="${BATS_TEST_DIRNAME}/../../bin/focal"
    TMP_DIR=$(mktemp -d)
    export XDG_CACHE_HOME="$TMP_DIR/cache"
    cd "$TMP_DIR"

    # A local bare repository stands in for a remote GitHub repository
    git init -q -b main src
    git -C src config user.email "test@test.com"
    git -C src config user.name "Test User"
    echo "remote readme" > src/README.md
    git -C src add .
    git -C src commit -qm "init"
    git clone -q --bare src remote.git
}

teardown() {
    cd "$ORIG_DIR"
    rm -rf "$TMP_DIR"
}

@test "--repo clones the remote and runs the command in the cached checkout" {
    run "$FOCAL_BIN" --repo "$TMP_DIR/remote.git" files README.md

    [ "$status" -eq 0 ]
    [[ "$output" == *"remote readme"* ]]
}

@test "--repo falls back to the cached copy when the remote is unreachable" {
    run "$FOCAL_BIN" --repo "$TMP_DIR/remote.git" files README.md
    [ "$status" -eq 0 ]

    mv remote.git remote.gone
    run "$FOCAL_BIN" --repo "$TMP_DIR/remote.git" files README.md

    [ "$status" -eq 0 ]
    [[ "$output" == *"using the cached copy"* ]]
    [[ "$output" == *"remote readme"* ]]
}

@test "global flag parsing stops at --" {
    cd src

    run "$FOCAL_BIN" diff -- --repo

    [ "$status" -eq 0 ]
    [[ "$output" != *"--repo requires an argument"* ]]
}

@test "cleanup_stale_cached_repos keeps a fresh cache entry without a marker" {
    source "${BATS_TEST_DIRNAME}/../../lib/core.sh"
    cache_dir="$TMP_DIR/repos"
    in_progress="$cache_dir/cloning_repo"
    mkdir -p "$in_progress/.git"

    run cleanup_stale_cached_repos "$cache_dir" 30 true

    [ "$status" -eq 0 ]
    [ -d "$in_progress" ]
}
