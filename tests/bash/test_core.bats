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

@test "parity: bash and python error formatting without hint match identically" {
    export NO_COLOR=1
    run status_error "file not found"
    bash_out="$output"

    py_out=$("$PYTHON_EXEC" -c 'import os; os.environ["NO_COLOR"]="1"; from focal.errors import format_error; print(format_error("file not found"))')
    [ "$bash_out" = "$py_out" ]
}

@test "parity: bash and python error formatting with hint match identically" {
    export NO_COLOR=1
    run bash -c "source '${BATS_TEST_DIRNAME}/../../lib/core.sh'; status_error 'invalid argument' && status_hint 'see --help'"
    bash_out="$output"

    py_out=$("$PYTHON_EXEC" -c 'import os; os.environ["NO_COLOR"]="1"; from focal.errors import format_error; print(format_error("invalid argument", hint="see --help"))')
    [ "$bash_out" = "$py_out" ]
}

@test "parity: bash and python warning formatting with hint match identically" {
    export NO_COLOR=1
    run bash -c "source '${BATS_TEST_DIRNAME}/../../lib/core.sh'; status_warn 'stale cache' && status_hint 'run refresh'"
    bash_out="$output"

    py_out=$("$PYTHON_EXEC" -c 'import os; os.environ["NO_COLOR"]="1"; from focal.errors import format_warning; print(format_warning("stale cache", hint="run refresh"))')
    [ "$bash_out" = "$py_out" ]
}

@test "noise config from lib/noise.json populates FOCAL_NOISE_EXTS and FOCAL_NOISE_FILES" {
    [ "${#FOCAL_NOISE_EXTS[@]}" -gt 0 ]
    [ "${#FOCAL_NOISE_FILES[@]}" -gt 0 ]

    # Verify key extensions and lockfiles are present
    [[ " ${FOCAL_NOISE_EXTS[*]} " =~ [[:space:]]parquet[[:space:]] ]]
    [[ " ${FOCAL_NOISE_EXTS[*]} " =~ [[:space:]]svg[[:space:]] ]]
    [[ " ${FOCAL_NOISE_FILES[*]} " =~ [[:space:]]uv.lock[[:space:]] ]]
    [[ " ${FOCAL_NOISE_FILES[*]} " =~ [[:space:]]Cargo.lock[[:space:]] ]]
}

@test "get_existing_core_manifests discovers existing manifests" {
    tmpdir=$(mktemp -d)
    touch "$tmpdir/README.md"
    touch "$tmpdir/Cargo.toml"
    touch "$tmpdir/other.txt"

    cd "$tmpdir"
    run get_existing_core_manifests
    rm -rf "$tmpdir"

    [ "$status" -eq 0 ]
    [[ "$output" == *"README.md"* ]]
    [[ "$output" == *"Cargo.toml"* ]]
    [[ "$output" != *"other.txt"* ]]
    [[ "$output" != *"package.json"* ]]
}

@test "get_file_metadata formats size and MIME type" {
    tmpdir=$(mktemp -d)
    echo "sample test content" > "$tmpdir/sample.txt"

    run get_file_metadata "$tmpdir/sample.txt"
    rm -rf "$tmpdir"

    [ "$status" -eq 0 ]
    [[ "$output" == *"Size: "* ]]
    [[ "$output" == *"Metadata: "* ]]
}

@test "status_info, status_done, status_add, and status_skip output formatted messages" {
    run status_info "info text"
    [ "$status" -eq 0 ]
    [[ "$output" == *"info: info text"* ]]

    run status_done "task complete"
    [ "$status" -eq 0 ]
    [[ "$output" == *"done: task complete"* ]]

    run status_add "new_file.txt"
    [ "$status" -eq 0 ]
    [[ "$output" == *"added: new_file.txt"* ]]

    run status_skip "ignored_file.txt"
    [ "$status" -eq 0 ]
    [[ "$output" == *"skipped: ignored_file.txt"* ]]
}

@test "should_use_color respects NO_COLOR and TERM=dumb" {
    NO_COLOR=1 run should_use_color 2
    [ "$status" -ne 0 ]

    TERM=dumb run should_use_color 2
    [ "$status" -ne 0 ]
}

@test "lib/preview.sh --issue formats issue preview markdown correctly" {
    tmpdir=$(mktemp -d)
    cat <<'EOF' > "$tmpdir/issues.json"
[
  {
    "number": 42,
    "title": "Add FZF Preview Support",
    "body": "Previewing should be fast and unified."
  }
]
EOF

    run "${BATS_TEST_DIRNAME}/../../lib/preview.sh" --issue 42 "$tmpdir/issues.json"
    rm -rf "$tmpdir"

    [ "$status" -eq 0 ]
    [[ "$output" == *"Add FZF Preview Support"* ]]
    [[ "$output" == *"Previewing should be fast and unified."* ]]
}

@test "is_noise_file matches multi-part extensions like min.js and min.css" {
    run is_noise_file "dist/bundle.min.js"
    [ "$status" -eq 0 ]

    run is_noise_file "styles/theme.min.css"
    [ "$status" -eq 0 ]

    run is_noise_file "src/regular.js"
    [ "$status" -ne 0 ]
}

@test "is_noise_file handles filenames with special characters safely" {
    run is_noise_file "component[1].tsx"
    [ "$status" -ne 0 ]

    run is_noise_file "package-lock.json"
    [ "$status" -eq 0 ]
}

@test "format_file_for_llm omits min.js and min.css files as noise" {
    tmpdir=$(mktemp -d)
    echo "var x=1;" > "$tmpdir/bundle.min.js"

    run format_file_for_llm "$tmpdir/bundle.min.js"
    rm -rf "$tmpdir"

    [ "$status" -eq 11 ]
    [[ "$output" == *"[asset/noise file omitted:"* ]]
}
