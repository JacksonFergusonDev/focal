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
