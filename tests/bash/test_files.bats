#!/usr/bin/env bats

setup() {
    FILES_BIN="${BATS_TEST_DIRNAME}/../../libexec/files"
    TMP_DIR=$(mktemp -d)
    cd "$TMP_DIR"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "files -h prints usage and options" {
    run "$FILES_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal files"* ]]
    [[ "$output" == *"-a, --all"* ]]
    [[ "$output" == *"-s, --staged"* ]]
    [[ "$output" == *"-m, --modified"* ]]
}

@test "files with single file outputs adaptive single-file message" {
    echo "print('hello')" > test.py

    run "$FILES_BIN" test.py

    [ "$status" -eq 0 ]
    [[ "$output" == *"Copied test.py to clipboard"* ]]
}

@test "files with multiple files outputs general context message" {
    echo "print('hello 1')" > file1.py
    echo "print('hello 2')" > file2.py

    run "$FILES_BIN" file1.py file2.py

    [ "$status" -eq 0 ]
    [[ "$output" == *"Copied context"* ]]
}
