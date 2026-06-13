#!/bin/bash
#
# Install knot agent-skills documentation
#
# Usage:
#   1. Local installation:
#      bash scripts/install-agent-skills.sh
#      bash scripts/install-agent-skills.sh /custom/path
#
#   2. Download and install in one command:
#      curl -fsSL https://raw.githubusercontent.com/user/knot/master/scripts/install-agent-skills.sh | bash
#
#   3. Specify custom directory:
#      curl -fsSL https://raw.githubusercontent.com/user/knot/master/scripts/install-agent-skills.sh | bash -s /my/custom/path
#

set -e

# Configuration
REPO_URL="${REPO_URL:-https://raw.githubusercontent.com/raultov/knot/master}"
TARGET_DIR="${1:-.knot-agent-skills}"
TEMP_DIR=$(mktemp -d)
TARBALL="$TEMP_DIR/agent-skills.tar.gz"

# Allow local source for development/testing
if [ -f ".knot-agent-skills.tar.gz" ]; then
  # Use local tarball if it exists
  TARBALL=".knot-agent-skills.tar.gz"
  SKIP_DOWNLOAD=1
fi

# Color output (printf interprets \033 directly — no shell-specific extensions).
RED=$(printf '\033[0;31m')
GREEN=$(printf '\033[0;32m')
YELLOW=$(printf '\033[1;33m')
BLUE=$(printf '\033[0;34m')
NC=$(printf '\033[0m') # No Color

trap "rm -rf $TEMP_DIR" EXIT

printf '%b📦 Installing knot agent-skills documentation%b\n' "$BLUE" "$NC"
printf '%b   Target: %b%s%b\n\n' "$NC" "$GREEN" "$TARGET_DIR" "$NC"

# Download the tarball (skip if using local source)
if [ -z "$SKIP_DOWNLOAD" ]; then
  printf '%bDownloading%b agent-skills... ' "$YELLOW" "$NC"
  if curl -fsSL "${REPO_URL}/.knot-agent-skills.tar.gz" -o "$TARBALL"; then
    printf '%b✓%b\n' "$GREEN" "$NC"
  else
    printf '%b✗%b\n' "$RED" "$NC"
    printf '%bError: Could not download agent-skills from %s%b\n' "$RED" "$REPO_URL" "$NC"
    exit 1
  fi
else
  printf '%bUsing%b local tarball... ' "$YELLOW" "$NC"
  printf '%b✓%b\n' "$GREEN" "$NC"
fi

# Create target directory
mkdir -p "$TARGET_DIR"

# Extract tarball
printf '%bExtracting%b files... ' "$YELLOW" "$NC"
if tar -xzf "$TARBALL" -C "$TARGET_DIR" --strip-components=2; then
  printf '%b✓%b\n' "$GREEN" "$NC"
else
  printf '%b✗%b\n' "$RED" "$NC"
  printf '%bError: Could not extract files%b\n' "$RED" "$NC"
  exit 1
fi

# Verify files
printf '%bVerifying%b files... ' "$YELLOW" "$NC"
files_found=0
files_expected=6
for file in search.md callers.md explore.md deps.md repos.md workflows.md; do
  if [ -f "$TARGET_DIR/$file" ]; then
    ((files_found++)) || true
  fi
done

if [ $files_found -eq $files_expected ]; then
  printf '%b✓%b\n' "$GREEN" "$NC"
else
  printf '%b⚠%b (found %d/%d files)\n' "$YELLOW" "$NC" "$files_found" "$files_expected"
fi

printf '\n'
printf '%b✅ Installation complete!%b\n' "$GREEN" "$NC"
printf '\n'
printf '%b📖 Available documentation:%b\n' "$BLUE" "$NC"
printf '   • %s/search.md       — Semantic code discovery\n' "$TARGET_DIR"
printf '   • %s/callers.md      — Reverse dependency lookup\n' "$TARGET_DIR"
printf '   • %s/explore.md      — File anatomy discovery\n' "$TARGET_DIR"
printf '   • %s/deps.md         — Repository dependency graph\n' "$TARGET_DIR"
printf '   • %s/repos.md        — Indexed repository inventory\n' "$TARGET_DIR"
printf '   • %s/workflows.md    — Common patterns & best practices\n' "$TARGET_DIR"
printf '\n'
printf '%b🚀 Quick start:%b\n' "$BLUE" "$NC"
printf '   cat %s/search.md\n' "$TARGET_DIR"
printf '   knot search "your query"\n'
printf '   knot explore "src/file.ts"\n'
printf '   knot callers "EntityName"\n'
printf '   knot deps my-app --reverse\n'
printf '   knot repos\n'
printf '\n'
printf '%b💡 Pro tip:%b\n' "$BLUE" "$NC"
printf '   alias knot-docs='"'"'less %s'"'"'\n' "$TARGET_DIR"
printf '   knot-docs/search.md\n'
