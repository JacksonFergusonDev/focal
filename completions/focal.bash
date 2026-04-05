_focal_completions() {
  if [ "${COMP_CWORD}" -eq 1 ]; then
    local cur_word="${COMP_WORDS[COMP_CWORD]}"
    local commands

    # Declare and assign separately
    commands=$(focal --commands 2>/dev/null)

    # Safely read the newline-separated output into the COMPREPLY array
    mapfile -t COMPREPLY < <(compgen -W "$commands" -- "$cur_word")
  fi
}
