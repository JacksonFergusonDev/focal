#!/usr/bin/env bats

setup() {
    REPO_DIR="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd)"
    # Prefer the system bash so macOS exercises Bash 3.2
    BASH_BIN=/bin/bash
    [ -x "$BASH_BIN" ] || BASH_BIN=$(command -v bash)
}

@test "bash completion registers itself for focal" {
    run "$BASH_BIN" -c "source '$REPO_DIR/completions/focal.bash' && complete -p focal"

    [ "$status" -eq 0 ]
    [[ "$output" == *"-F _focal_completions focal"* ]]
}

@test "bash completion suggests matching subcommands" {
    run env PATH="$REPO_DIR/bin:$PATH" "$BASH_BIN" -c "
        source '$REPO_DIR/completions/focal.bash'
        COMP_WORDS=(focal di)
        COMP_CWORD=1
        _focal_completions
        printf '%s\n' \"\${COMPREPLY[@]}\"
    "

    [ "$status" -eq 0 ]
    [ "$output" = "diff" ]
}
