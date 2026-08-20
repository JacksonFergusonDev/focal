#!/usr/bin/env bats

setup() {
    DIFF_BIN="${BATS_TEST_DIRNAME}/../../libexec/diff"
    TMP_REPO=$(mktemp -d)
    cd "$TMP_REPO"
    git init -q
    git config user.name "Test User"
    git config user.email "test@example.com"
}

teardown() {
    rm -rf "$TMP_REPO"
}

@test "diff -h prints usage and options" {
    run "$DIFF_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal diff [flags] [--] [path...]"* ]]
    [[ "$output" == *"--staged, --cached"* ]]
    [[ "$output" == *"-a, --all"* ]]
    [[ "$output" == *"--thick"* ]]
}

@test "diff ignores uv.lock when unstaged" {
    echo "initial" > main.py
    echo "lock v1" > uv.lock
    git add . && git commit -q -m "initial commit"

    echo "code change" >> main.py
    echo "lock v2" >> uv.lock

    run "$DIFF_BIN"

    [ "$status" -eq 0 ]
    [[ "$output" == *"main.py"* ]]
    [[ "$output" == *"code change"* ]]
    [[ "$output" != *"uv.lock"* ]]
    [[ "$output" != *"lock v2"* ]]
}

@test "diff outputs no changes detected when only uv.lock is modified" {
    echo "initial" > main.py
    echo "lock v1" > uv.lock
    git add . && git commit -q -m "initial commit"

    echo "lock v2" >> uv.lock

    run "$DIFF_BIN"

    [ "$status" -eq 0 ]
    [[ "$output" == *"No changes detected in working tree."* ]]
}

@test "diff --staged ignores uv.lock" {
    echo "initial" > main.py
    echo "lock v1" > uv.lock
    git add . && git commit -q -m "initial commit"

    echo "code change" >> main.py
    echo "lock v2" >> uv.lock
    git add main.py uv.lock

    run "$DIFF_BIN" --staged

    [ "$status" -eq 0 ]
    [[ "$output" == *"main.py"* ]]
    [[ "$output" == *"code change"* ]]
    [[ "$output" != *"uv.lock"* ]]
    [[ "$output" != *"lock v2"* ]]
}

@test "diff --all ignores untracked and modified uv.lock" {
    echo "initial" > main.py
    git add . && git commit -q -m "initial commit"

    echo "code change" >> main.py
    echo "lock untracked" > uv.lock
    echo "package lock untracked" > package-lock.json

    run "$DIFF_BIN" --all

    [ "$status" -eq 0 ]
    [[ "$output" == *"main.py"* ]]
    [[ "$output" == *"code change"* ]]
    [[ "$output" != *"uv.lock"* ]]
    [[ "$output" != *"package-lock.json"* ]]
}

@test "diff allows explicitly diffing uv.lock when passed as path argument" {
    echo "initial" > main.py
    echo "lock v1" > uv.lock
    git add . && git commit -q -m "initial commit"

    echo "code change" >> main.py
    echo "lock v2" >> uv.lock

    run "$DIFF_BIN" uv.lock

    [ "$status" -eq 0 ]
    [[ "$output" == *"uv.lock"* ]]
    [[ "$output" == *"lock v2"* ]]
    [[ "$output" != *"main.py"* ]]
}
