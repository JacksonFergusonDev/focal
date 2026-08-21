#!/usr/bin/env bats

setup() {
    source "${BATS_TEST_DIRNAME}/../../lib/core.sh"
}

@test "require_cmd succeeds silently if command exists" {
    # Catches the exit code and stdout/stderr
    run require_cmd "ls"

    # Assert exit code was 0
    [ "$status" -eq 0 ]

    # Assert nothing was printed
    [ "$output" = "" ]
}

@test "require_cmd fails and outputs error if command is missing" {
    # Using a definitively non-existent command
    run require_cmd "quantum_flux_capacitor"

    # Assert exit code was 1 (failure)
    [ "$status" -eq 1 ]

    # Assert stderr message matches expected pattern
    [[ "$output" == *"error: 'quantum_flux_capacitor' is not installed"* ]]
}

@test "_get_clipboard_cmd detects available clipboard manager" {
    run _get_clipboard_cmd

    [ "$status" -eq 0 ]

    case "$output" in
        "pbcopy"|"wl-copy"|"xclip -selection clipboard"|"xsel --clipboard --input"|"")
            true ;; # Pass
        *)
            echo "Unexpected output: $output" >&3
            false ;; # Fail
    esac
}

@test "format_files_for_llm preserves double newlines between multiple files" {
    tmpdir=$(mktemp -d)
    echo "content1" > "$tmpdir/file1.txt"
    echo "content2" > "$tmpdir/file2.txt"

    run format_files_for_llm "$tmpdir/file1.txt" "$tmpdir/file2.txt"
    rm -rf "$tmpdir"

    [ "$status" -eq 0 ]
    [[ "$output" == *"content1"* ]]
    [[ "$output" == *"content2"* ]]
    [[ "$output" == *"content1"*$'\n```\n\n# '*"content2"* ]]
}

@test "resolve_path_args handles files, directories with trailing slash, and deduplicates" {
    tmpdir=$(mktemp -d)
    mkdir -p "$tmpdir/pkg"
    touch "$tmpdir/pkg/mod1.py"
    touch "$tmpdir/pkg/mod2.py"
    touch "$tmpdir/root.txt"

    cd "$tmpdir"
    run resolve_path_args "pkg/" "root.txt" "pkg/mod1.py"
    rm -rf "$tmpdir"

    [ "$status" -eq 0 ]
    [[ "$output" == *"pkg/mod1.py"* ]]
    [[ "$output" == *"pkg/mod2.py"* ]]
    [[ "$output" == *"root.txt"* ]]

    # Verify deduplication (mod1.py should only appear once)
    count=$(echo "$output" | grep -c "pkg/mod1.py" || true)
    [ "$count" -eq 1 ]
}

@test "resolve_path_args errors if directory passed without trailing slash" {
    tmpdir=$(mktemp -d)
    mkdir -p "$tmpdir/pkg"
    touch "$tmpdir/pkg/mod1.py"

    cd "$tmpdir"
    run resolve_path_args "pkg"
    rm -rf "$tmpdir"

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: 'pkg' is a directory; append a trailing slash (e.g. 'pkg/') to select its contents"* ]]
}

@test "resolve_path_args errors if path does not exist" {
    run resolve_path_args "nonexistent_file.py"

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: file not found: nonexistent_file.py"* ]]
}

@test "resolve_path_args errors if directory with trailing slash does not exist" {
    run resolve_path_args "nonexistent_dir/"

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: directory not found: nonexistent_dir"* ]]
}

@test "status_error, status_warn, and status_hint print formatted messages" {
    run status_error "something broke"
    [ "$status" -eq 0 ]
    [[ "$output" == *"error: something broke"* ]]

    run status_warn "heads up"
    [ "$status" -eq 0 ]
    [[ "$output" == *"warning: heads up"* ]]

    run status_hint "try again"
    [ "$status" -eq 0 ]
    [[ "$output" == *"hint: try again"* ]]
}

@test "die outputs error, hint and exits with status code" {
    run die "critical failure" "check configuration" 42
    [ "$status" -eq 42 ]
    [[ "$output" == *"error: critical failure"* ]]
    [[ "$output" == *"hint: check configuration"* ]]
}
