#!/usr/bin/env bash

# ==========================================
# Core Library: Shared Utilities & Constants
# ==========================================

# ------------------------------------------
# Global Noise/Asset Filtering
# ------------------------------------------

FOCAL_NOISE_EXTS=(
  # Compiled/Binary Data
  "parquet" "pkl" "sqlite" "db" "npy" "npz" "h5" "hdf5" "fits" "data" "nc"
  # Media & Assets
  "svg" "png" "jpg" "jpeg" "gif" "ico" "webp" "pdf" "mp4" "webm" "mov" "avi" "mkv" "mp3" "wav"
  # Frontend build artifacts
  "min.js" "min.css" "map"
  # Archives, Lockfiles & Compiled
  "zip" "tar" "gz" "xz" "bz2" "whl" "pyc" "bin" "exe" "so" "dylib" "dll" "lock"
)

FOCAL_NOISE_REGEX="^($(
  IFS='|'
  echo "${FOCAL_NOISE_EXTS[*]}"
))$"

FD_NOISE_FLAGS=()
for ext in "${FOCAL_NOISE_EXTS[@]}"; do
  FD_NOISE_FLAGS+=("-E" "*.$ext")
done

# ------------------------------------------
# Environment & Path Resolution
# ------------------------------------------

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

# ------------------------------------------
# Utility Functions
# ------------------------------------------

# Command Validation
require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: '$cmd' is not installed or not in PATH." >&2
    exit 1
  fi
}

# ------------------------------------------
# Clipboard Detection & Output Handling
# ------------------------------------------

# Clipboard Detection (Internal)
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

# Standardized Output Routine
output_and_copy() {
  local content="$1"
  local base_msg="$2"
  read -r -a clip_cmd <<<"$(_get_clipboard_cmd)"

  # Centralized LLM boundary and metadata injection
  local payload="<context>"$'\n'
  payload+="> [!NOTE]"$'\n'
  payload+="> The following is an automated extraction of the user's local codebase environment."$'\n'
  payload+="> Use this metadata, directory structure, and file contents to inform your response to the user's subsequent prompt."$'\n\n'

  payload+="# Workspace Context"$'\n'
  payload+="PWD: $(pwd)"$'\n'
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    payload+="Git Branch: $(git branch --show-current)"$'\n'
    payload+="Git Commit: $(git rev-parse --short HEAD)"$'\n'
  fi
  payload+=$'\n---\n\n'
  payload+="${content}"$'\n'
  payload+="</context>"$'\n'

  # Calculate rough token estimate (1 token ~= 4 chars)
  local char_count=${#payload}
  local approx_tokens=$((char_count / 4))

  # Inject the token estimate into the success message
  local success_msg="${base_msg} (~${approx_tokens} tokens)"

  # Check if the array has elements
  if [ ${#clip_cmd[@]} -gt 0 ]; then
    # Use %s to prevent escape sequence expansion
    printf "%s" "$payload" | "${clip_cmd[@]}"
    echo "$success_msg"
  else
    printf "%s" "$payload"
  fi
}

# ------------------------------------------
# File Selection & LLM Formatting
# ------------------------------------------

# File UI & Parsing
interactive_file_select() {
  local prompt="${1:-Select file: }"
  shift || true
  local fzf_args=("$@")

  # Dynamically build exclusions if the FOCAL_EXCLUDE_FILES array is populated
  local extra_fd_args=()
  if [ -n "${FOCAL_EXCLUDE_FILES+x}" ] && [ "${#FOCAL_EXCLUDE_FILES[@]}" -gt 0 ]; then
    for excl in "${FOCAL_EXCLUDE_FILES[@]}"; do
      extra_fd_args+=("--exclude" "$excl")
    done
  fi

  fd --type f --hidden --exclude .git "${FD_NOISE_FLAGS[@]}" "${extra_fd_args[@]}" | fzf "${fzf_args[@]}" \
    --prompt="$prompt" \
    --bind "ctrl-a:select-all,ctrl-d:deselect-all" \
    --preview "${REPO_ROOT}/lib/preview.sh {}" || true
}

format_file_for_llm() {
  local file="$1"
  local ext="${file##*.}"
  local ext_lower
  ext_lower=$(echo "$ext" | tr '[:upper:]' '[:lower:]')
  local content=""

  # Intercept empty files immediately (e.g., __init__.py)
  if [ ! -s "$file" ]; then
    content="[empty file]"
  elif [[ $ext_lower == "ipynb" ]]; then
    content=$("$PYTHON_EXEC" -m focal.notebook "$file")
  elif [[ $ext_lower =~ $FOCAL_NOISE_REGEX ]]; then
    content="[asset/noise file omitted: $file]"
  else
    local mime_enc
    mime_enc=$(file -b --mime-encoding "$file" 2>/dev/null || echo "binary")

    if [[ $mime_enc == "binary" ]]; then
      content="[binary file omitted: $file]"
    else
      # Global Text Failsafe
      local total_lines
      total_lines=$(wc -l <"$file" | tr -d ' ')

      if [ "$total_lines" -gt 1500 ]; then
        content=$(head -n 1500 "$file")
        content+=$'\n\n...[file truncated: showing first 1500 of '"$total_lines"' lines]'
      else
        content=$(cat "$file")
      fi
    fi
  fi

  # Return formatted markdown block
  printf "# %s\n\`\`\`%s\n%s\n\`\`\`\n\n" "$file" "$ext" "$content"
}

# ------------------------------------------
# Repository Context Generation
# ------------------------------------------

generate_repo_tree() {
  if command -v tree >/dev/null 2>&1; then
    tree -a -I '.git|node_modules|.venv|__pycache__|dist|build' --gitignore 2>/dev/null
  else
    fd --hidden --exclude .git --type f | sed 's|[^/]*/|  |g'
  fi
}

_print_context_file() {
  local file="$1"
  local lang="$2"
  local max_lines=500

  if [ ! -f "$file" ]; then
    return
  fi

  echo "# $file"
  echo "\`\`\`$lang"
  head -n "$max_lines" "$file"
  echo '```'
  echo
}

generate_project_context() {
  echo "# Repository Tree"
  echo '```text'
  generate_repo_tree
  echo '```'
  echo

  if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "# Git Info"
    echo '```text'
    echo "Branch: $(git branch --show-current)"
    echo "Commit: $(git rev-parse --short HEAD)"
    echo
    echo "Modified files:"
    git status --porcelain
    echo '```'
    echo
  fi

  _print_context_file "pyproject.toml" "toml"
  _print_context_file "package.json" "json"
  _print_context_file "requirements.txt" "text"
  _print_context_file "README.md" "markdown"
}
