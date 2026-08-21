#!/usr/bin/env bats

setup() {
    CONTEXT_BIN="${BATS_TEST_DIRNAME}/../../libexec/context"
    TMP_DIR=$(mktemp -d)
    cd "$TMP_DIR"
    git init -q
    git config user.email "test@test.com"
    git config user.name "Test User"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "context -h prints usage and options" {
    run "$CONTEXT_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal context"* ]]
    [[ "$output" == *"[files/dirs...] | [flags]"* ]]
    [[ "$output" == *"-c, --core"* ]]
    [[ "$output" == *"-a, --all"* ]]
}

@test "context with path args in --core mode includes preselected files" {
    mkdir -p src
    echo "print('src code')" > src/app.py
    echo "# README" > README.md
    git add .
    git commit -qm "init"

    run "$CONTEXT_BIN" -c src/

    [ "$status" -eq 0 ]
    [[ "$output" == *"# README.md"* ]]
    [[ "$output" == *"# src/app.py"* ]]
    [[ "$output" == *"print('src code')"* ]]
}

@test "context with specific file args in --core mode includes them" {
    echo "val = 42" > config.py
    echo "# README" > README.md
    git add .
    git commit -qm "init"

    run "$CONTEXT_BIN" -c config.py

    [ "$status" -eq 0 ]
    [[ "$output" == *"# README.md"* ]]
    [[ "$output" == *"# config.py"* ]]
    [[ "$output" == *"val = 42"* ]]
}

@test "context with path args in --all mode includes files" {
    mkdir -p src
    echo "print('hello')" > src/main.py
    git add .
    git commit -qm "init"

    run "$CONTEXT_BIN" -a src/

    if [ "$status" -ne 0 ]; then
        echo "TEST FAILED WITH STATUS: $status" >&2
        echo "TEST OUTPUT: $output" >&2
    fi

    [ "$status" -eq 0 ]
    [[ "$output" == *"# src/main.py"* ]]
}

@test "context with nonexistent path aborts with error" {
    run "$CONTEXT_BIN" nonexistent/

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: directory not found: nonexistent"* ]]
}

@test "context with directory missing trailing slash aborts with error" {
    mkdir -p src
    touch src/app.py

    run "$CONTEXT_BIN" src

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: 'src' is a directory; append a trailing slash (e.g. 'src/') to select its contents"* ]]
}
