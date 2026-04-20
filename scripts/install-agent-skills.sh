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

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

trap "rm -rf $TEMP_DIR" EXIT

echo -e "${BLUE}📦 Installing knot agent-skills documentation${NC}"
echo -e "   Target: ${GREEN}${TARGET_DIR}${NC}\n"

# Download the tarball (skip if using local source)
if [ -z "$SKIP_DOWNLOAD" ]; then
  echo -ne "${YELLOW}Downloading${NC} agent-skills... "
  if curl -fsSL "${REPO_URL}/.knot-agent-skills.tar.gz" -o "$TARBALL"; then
    echo -e "${GREEN}✓${NC}"
  else
    echo -e "${RED}✗${NC}"
    echo -e "${RED}Error: Could not download agent-skills from ${REPO_URL}${NC}"
    exit 1
  fi
else
  echo -ne "${YELLOW}Using${NC} local tarball... "
  echo -e "${GREEN}✓${NC}"
fi

# Create target directory
mkdir -p "$TARGET_DIR"

# Extract tarball
echo -ne "${YELLOW}Extracting${NC} files... "
if tar -xzf "$TARBALL" -C "$TARGET_DIR" --strip-components=2; then
  echo -e "${GREEN}✓${NC}"
else
  echo -e "${RED}✗${NC}"
  echo -e "${RED}Error: Could not extract files${NC}"
  exit 1
fi

# Verify files
echo -ne "${YELLOW}Verifying${NC} files... "
files_found=0
for file in search.md callers.md explore.md workflows.md; do
  if [ -f "$TARGET_DIR/$file" ]; then
    ((files_found++))
  fi
done

if [ $files_found -eq 4 ]; then
  echo -e "${GREEN}✓${NC}"
else
  echo -e "${YELLOW}⚠${NC} (found $files_found/4 files)"
fi

echo ""
echo -e "${GREEN}✅ Installation complete!${NC}"
echo ""
echo -e "📖 ${BLUE}Available documentation:${NC}"
echo "   • ${TARGET_DIR}/search.md       — Semantic code discovery"
echo "   • ${TARGET_DIR}/callers.md      — Reverse dependency lookup"
echo "   • ${TARGET_DIR}/explore.md      — File anatomy discovery"
echo "   • ${TARGET_DIR}/workflows.md    — Common patterns & best practices"
echo ""
echo -e "🚀 ${BLUE}Quick start:${NC}"
echo "   cat ${TARGET_DIR}/search.md"
echo "   knot search \"your query\""
echo "   knot explore \"src/file.ts\""
echo "   knot callers \"EntityName\""
echo ""
echo -e "💡 ${BLUE}Pro tip:${NC}"
echo "   alias knot-docs='less ${TARGET_DIR}'"
echo "   knot-docs/search.md"
