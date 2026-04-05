#!/usr/bin/env bash

# 1. Path Resolution
LIB_DIR="$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")" && pwd)"
REPO_ROOT="$(realpath "${LIB_DIR}/..")"

# Explicitly set PYTHONPATH so Python can resolve focal globally
export PYTHONPATH="${REPO_ROOT}:${PYTHONPATH:-}"

# 2. Smart Python Resolution
if [ -f "${REPO_ROOT}/.venv/bin/python3" ]; then
  PYTHON_EXEC="${REPO_ROOT}/.venv/bin/python3"
elif [ -f "${REPO_ROOT}/libexec/bin/python3" ]; then
  PYTHON_EXEC="${REPO_ROOT}/libexec/bin/python3"
else
  # shellcheck disable=SC2034
  PYTHON_EXEC="python3"
fi

# 2. Command Validation
require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: '$cmd' is not installed or not in PATH." >&2
    exit 1
  fi
}

# 3. Clipboard Detection (Internal)
_get_clipboard_cmd() {
  if command -v pbcopy >/dev/null 2>&1; then
    echo "pbcopy"
  elif command -v wl-copy >/dev/null 2>&1; then
    echo "wl-copy"
  elif command -v xclip >/dev/null 2>&1; then
    echo "xclip -selection clipboard"
  elif command -v xsel >/dev/null 2>&1; then
    echo "xsel --clipboard --input"
  else
    echo ""
  fi
}

# 4. Standardized Output Routine
output_and_copy() {
  local content="$1"
  local base_msg="$2"
  read -r -a clip_cmd <<<"$(_get_clipboard_cmd)"

  # Calculate rough token estimate (1 token ~= 4 chars)
  local char_count=${#content}
  local approx_tokens=$((char_count / 4))

  # Inject the token estimate into the success message
  local success_msg="${base_msg} (~${approx_tokens} tokens)"

  # Check if the array has elements
  if [ ${#clip_cmd[@]} -gt 0 ]; then
    # Use %s to prevent escape sequence expansion
    printf "%s" "$content" | "${clip_cmd[@]}"
    echo "$success_msg"
  else
    printf "%s" "$content"
  fi
}

# 5. File UI & Parsing
interactive_file_select() {
  local prompt="${1:-Select file: }"
  shift || true
  local fzf_args=("$@")

  fd --type f --hidden --exclude .git | fzf "${fzf_args[@]}" \
    --prompt="$prompt" \
    --bind "ctrl-a:select-all,ctrl-d:deselect-all" \
    --preview "${REPO_ROOT}/lib/preview.sh {}" || true
}

format_file_for_llm() {
  local file="$1"
  local ext="${file##*.}"
  local content=""

  if [[ $ext == "ipynb" ]]; then
    content=$("$PYTHON_EXEC" -m focal.notebook "$file")
  elif file "$file" | grep -q binary; then
    content="[binary file omitted: $file]"
  else
    content=$(cat "$file")
  fi

  # Return formatted markdown block
  printf "# %s\n\`\`\`%s\n%s\n\`\`\`\n\n" "$file" "$ext" "$content"
}

# 6. Context Generation
generate_repo_tree() {
  if command -v tree >/dev/null 2>&1; then
    tree -a -I '.git|node_modules|.venv|__pycache__|dist|build' --gitignore 2>/dev/null
  else
    fd --hidden --exclude .git --type f | sed 's|[^/]*/|  |g'
  fi
}
