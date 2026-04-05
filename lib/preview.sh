#!/usr/bin/env bash
set -euo pipefail

# 1. Source core to get $PYTHON_EXEC and $REPO_ROOT
# We use a relative path to find core.sh since they are in the same directory
LIB_DIR="$(cd "$(dirname "$(realpath "${BASH_SOURCE[0]}")")" && pwd)"
source "${LIB_DIR}/core.sh"

TARGET="$1"
[ ! -f "$TARGET" ] && exit 0

EXT="${TARGET##*.}"

# 2. Parse Notebooks
if [[ $EXT == "ipynb" ]]; then
  # Use the synchronized Python environment to run the notebook parser
  "$PYTHON_EXEC" -m focal.notebook "$TARGET" 2>/dev/null | bat --language=markdown --style=numbers --color=always
  exit 0
fi

# 3. Detect Binaries
MIME_ENC=$(file -b --mime-encoding "$TARGET" 2>/dev/null || echo "binary")

if [[ $MIME_ENC == "binary" ]]; then
  echo -e "\033[1;34m[Binary / Image File Omitted from Preview]\033[0m\n"
  echo -e "\033[1mFile:\033[0m $TARGET"
  echo -e "\033[1mType:\033[0m $(file -b "$TARGET")"
  echo -e "\033[1mSize:\033[0m $(du -h "$TARGET" | awk '{print $1}')"
  exit 0
fi

# 4. Text Fallback
bat --style=numbers --color=always "$TARGET" 2>/dev/null || cat "$TARGET"
