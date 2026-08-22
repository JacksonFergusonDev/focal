#!/usr/bin/env bats

setup() {
    SEARCH_BIN="${BATS_TEST_DIRNAME}/../../libexec/search"
    SYMBOL_BIN="${BATS_TEST_DIRNAME}/../../libexec/symbol"
    API_BIN="${BATS_TEST_DIRNAME}/../../libexec/api"
    TREE_BIN="${BATS_TEST_DIRNAME}/../../libexec/tree"
    TMP_DIR=$(mktemp -d)
    cd "$TMP_DIR"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "search -h prints usage and options" {
    run "$SEARCH_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal search <pattern>"* ]]
}

@test "search without pattern errors with usage hint" {
    run "$SEARCH_BIN"

    [ "$status" -ne 0 ]
    [[ "$output" == *"missing search pattern"* ]]
}

@test "search with no matching results exits 0 and prints info message" {
    echo "hello world" > sample.txt

    run "$SEARCH_BIN" "non_existent_pattern_xyz"

    [ "$status" -eq 0 ]
    [[ "$output" == *"info: No matches found for 'non_existent_pattern_xyz'."* ]]
}

@test "search with matching pattern outputs matches" {
    echo "target match line" > sample.txt

    run "$SEARCH_BIN" "target match"

    [ "$status" -eq 0 ]
    [[ "$output" == *"target match line"* ]]
}

@test "symbol -h prints usage and options" {
    run "$SYMBOL_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal symbol <symbol>"* ]]
}

@test "symbol without symbol errors with usage hint" {
    run "$SYMBOL_BIN"

    [ "$status" -ne 0 ]
    [[ "$output" == *"missing symbol name"* ]]
}

@test "symbol with non-existent symbol exits 0 and prints info message" {
    echo "some file content" > sample.txt

    run "$SYMBOL_BIN" "NonExistentClassOrSymbol"

    [ "$status" -eq 0 ]
    [[ "$output" == *"info: No files found containing symbol 'NonExistentClassOrSymbol'."* ]]
}

@test "symbol with matching symbol extracts and formats file" {
    cat <<'EOF_PY' > sample.py
class TargetClass:
    def __init__(self):
        pass
EOF_PY

    run "$SYMBOL_BIN" "TargetClass"

    [ "$status" -eq 0 ]
    [[ "$output" == *"class TargetClass:"* ]]
    [[ "$output" == *"sample.py"* ]]
}

@test "api -h prints usage and options" {
    run "$API_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal api"* ]]
}

@test "api with no python files or definitions exits 0 and prints info message" {
    echo "plain text file" > sample.txt

    run "$API_BIN"

    [ "$status" -eq 0 ]
    [[ "$output" == *"info: No Python classes or functions found."* ]]
}

@test "api with python definitions extracts classes and functions" {
    cat <<'EOF_PY' > sample.py
class Greeter:
    def greet(self):
        return "hi"

def standalone_helper():
    pass
EOF_PY

    run "$API_BIN"

    [ "$status" -eq 0 ]
    [[ "$output" == *"class Greeter:"* ]]
    [[ "$output" == *"def standalone_helper():"* ]]
}

@test "tree -h prints usage and options" {
    run "$TREE_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal tree"* ]]
}

@test "tree generates directory tree" {
    mkdir -p sub/dir
    touch sub/dir/file.txt

    run "$TREE_BIN"

    [ "$status" -eq 0 ]
    [[ "$output" == *"sub"* ]]
    [[ "$output" == *"file.txt"* ]]
}
