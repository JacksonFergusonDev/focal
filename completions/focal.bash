_focal_completions() {
  if [ "${COMP_CWORD}" -eq 1 ]; then
    local cur_word="${COMP_WORDS[COMP_CWORD]}"
    local commands

    # Declare and assign separately
    commands=$(focal --commands 2>/dev/null)

    # Read line by line rather than with mapfile, which Bash 3.2 (macOS) lacks
    COMPREPLY=()
    local candidate
    while IFS= read -r candidate; do
      COMPREPLY+=("$candidate")
    done < <(compgen -W "$commands" -- "$cur_word")
  fi
}

complete -F _focal_completions focal
