#!/usr/bin/env bats

setup() {
    CI_FAIL_BIN="${BATS_TEST_DIRNAME}/../../libexec/ci-fail"
}

@test "ci-fail -h prints usage and options" {
    run "$CI_FAIL_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal ci-fail [options]"* ]]
    [[ "$output" == *"-a, --all"* ]]
    [[ "$output" == *"-h, --help"* ]]
}

@test "ci-fail rejects invalid arguments" {
    run "$CI_FAIL_BIN" --unknown-flag

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: unknown argument: --unknown-flag"* ]]
}
