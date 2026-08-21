#!/usr/bin/env bats

setup() {
    RELEASE_CONTEXT_BIN="${BATS_TEST_DIRNAME}/../../libexec/release-context"
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
