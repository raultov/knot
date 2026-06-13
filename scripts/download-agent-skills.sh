#!/bin/bash
#
# Download and extract knot agent-skills documentation
#
# Usage: curl -fsSL https://raw.githubusercontent.com/user/knot/master/scripts/download-agent-skills.sh | bash
# Or: curl -fsSL https://raw.githubusercontent.com/user/knot/master/.knot-agent.md | grep "^# DOWNLOAD SCRIPT" -A 999 | bash
#

set -e

# Color output (printf interprets \033 directly — no shell-specific extensions).
RED=$(printf '\033[0;31m')
GREEN=$(printf '\033[0;32m')
YELLOW=$(printf '\033[1;33m')
BLUE=$(printf '\033[0;34m')
NC=$(printf '\033[0m') # No Color

# Config
TARGET_DIR="${1:-.knot-agent-skills}"
GITHUB_REPO="${2:-https://raw.githubusercontent.com/user/knot/master}"

printf '%b📦 Downloading knot agent-skills documentation...%b\n' "$BLUE" "$NC"

# Create target directory
mkdir -p "$TARGET_DIR"

# Define files to download
files=(
  "search.md"
  "callers.md"
  "explore.md"
  "deps.md"
  "repos.md"
  "workflows.md"
)

# Base URL for documentation
BASE_URL="${GITHUB_REPO}/docs/agent-skills"

printf '%bDestination: %b%s%b\n\n' "$BLUE" "$GREEN" "$TARGET_DIR" "$NC"

# Download each file
downloaded=0
for file in "${files[@]}"; do
  printf '%bDownloading%b %s ... ' "$YELLOW" "$NC" "$file"

  if curl -fsSL "${BASE_URL}/${file}" -o "${TARGET_DIR}/${file}"; then
    printf '%b✓%b\n' "$GREEN" "$NC"
    downloaded=$((downloaded + 1))
  else
    printf '%b✗%b\n' "$RED" "$NC"
  fi
done

printf '\n'
printf '%b✅ Downloaded %d/%d files%b\n' "$GREEN" "$downloaded" "${#files[@]}" "$NC"
printf '\n'
printf '%b📖 Documentation files:%b\n' "$BLUE" "$NC"
printf '   - %s/search.md       (Semantic code discovery)\n' "$TARGET_DIR"
printf '   - %s/callers.md      (Reverse dependency lookup)\n' "$TARGET_DIR"
printf '   - %s/explore.md      (File anatomy discovery)\n' "$TARGET_DIR"
printf '   - %s/deps.md         (Repository dependency graph)\n' "$TARGET_DIR"
printf '   - %s/repos.md        (Indexed repository inventory)\n' "$TARGET_DIR"
printf '   - %s/workflows.md    (Common patterns & best practices)\n' "$TARGET_DIR"
printf '\n'
printf '%b🚀 Quick start:%b\n' "$BLUE" "$NC"
printf '   knot search "your query"\n'
printf '   knot explore "src/path/to/file.ts"\n'
printf '   knot callers "EntityName"\n'
printf '   knot deps my-app --reverse\n'
printf '   knot repos\n'
printf '\n'
printf '%b📖 Read the guides:%b\n' "$BLUE" "$NC"
printf '   less %s/search.md\n' "$TARGET_DIR"
printf '   less %s/workflows.md\n' "$TARGET_DIR"
