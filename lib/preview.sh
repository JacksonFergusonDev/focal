#!/usr/bin/env bash
set -euo pipefail

# 1. Source core to get $PYTHON_EXEC and $REPO_ROOT
# We use a relative path to find core.sh since they are in the same directory
LIB_DIR="$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")" && pwd)"
source "${LIB_DIR}/core.sh"

TARGET="$1"

# Helper for markdown rendering with bat/batcat fallback
_render_markdown() {
  if command -v bat >/dev/null 2>&1; then
    bat --language=markdown --style=numbers --color=always
  elif command -v batcat >/dev/null 2>&1; then
    batcat --language=markdown --style=numbers --color=always
  else
    cat
  fi
}

# 2. Parse special flags
if [[ $TARGET == "--stdin-markdown" ]]; then
  _render_markdown
  exit 0
fi

if [[ $TARGET == "--issue" ]]; then
  ISSUE_ID="$2"
  ISSUES_JSON="$3"
  jq -r --argjson id "$ISSUE_ID" '.[] | select(.number == $id) | if .body == "" or .body == null then "# \(.title)\n\n*No description provided.*" else "# \(.title)\n\n\(.body)" end' "$ISSUES_JSON" | _render_markdown
  exit 0
fi

[ ! -f "$TARGET" ] && exit 0

EXT="${TARGET##*.}"

# 3. Parse Notebooks
if [[ $EXT == "ipynb" ]]; then
  # Use the synchronized Python environment to run the notebook parser
  "$PYTHON_EXEC" -m focal notebook "$TARGET" 2>/dev/null | _render_markdown
  exit 0
fi

# 4. Parse PDFs
if [[ $EXT == "pdf" ]]; then
  "$PYTHON_EXEC" -m focal pdf "$TARGET" 2>/dev/null | _render_markdown
  exit 0
fi

# 5. Detect Noise / Binaries
MIME_ENC=$(file -b --mime-encoding "$TARGET" 2>/dev/null || echo "binary")

if is_noise_file "$TARGET" || [[ $MIME_ENC == "binary" ]]; then
  printf "%b\n\n" "${CYAN}${BOLD}[Binary / Asset File Omitted from Preview]${RESET}"
  printf "%b %s\n" "${BOLD}File:${RESET}" "$TARGET"
  printf "%b %s\n" "${BOLD}Type:${RESET}" "$(file -b "$TARGET" 2>/dev/null || echo "Unknown")"
  printf "%b %s\n" "${BOLD}Size:${RESET}" "$(ls -lh "$TARGET" 2>/dev/null | awk '{print $5}' || echo "0B")"
  exit 0
fi

# 6. Text Fallback
if command -v bat >/dev/null 2>&1; then
  bat --style=numbers --color=always "$TARGET" 2>/dev/null || cat "$TARGET"
elif command -v batcat >/dev/null 2>&1; then
  batcat --style=numbers --color=always "$TARGET" 2>/dev/null || cat "$TARGET"
else
  cat "$TARGET"
fi
