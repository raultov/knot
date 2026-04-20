#!/bin/bash
#
# Download and extract knot agent-skills documentation
#
# Usage: curl -fsSL https://raw.githubusercontent.com/user/knot/master/scripts/download-agent-skills.sh | bash
# Or: curl -fsSL https://raw.githubusercontent.com/user/knot/master/.knot-agent.md | grep "^# DOWNLOAD SCRIPT" -A 999 | bash
#

set -e

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Config
TARGET_DIR="${1:-.knot-agent-skills}"
GITHUB_REPO="${2:-https://raw.githubusercontent.com/user/knot/master}"

echo -e "${BLUE}📦 Downloading knot agent-skills documentation...${NC}"

# Create target directory
mkdir -p "$TARGET_DIR"

# Define files to download
files=(
  "search.md"
  "callers.md"
  "explore.md"
  "workflows.md"
)

# Base URL for documentation
BASE_URL="${GITHUB_REPO}/docs/agent-skills"

echo -e "${BLUE}Destination: ${GREEN}${TARGET_DIR}${NC}\n"

# Download each file
downloaded=0
for file in "${files[@]}"; do
  echo -ne "${YELLOW}Downloading${NC} $file ... "
  
  if curl -fsSL "${BASE_URL}/${file}" -o "${TARGET_DIR}/${file}"; then
    echo -e "${GREEN}✓${NC}"
    ((downloaded++))
  else
    echo -e "${RED}✗${NC}"
  fi
done

echo ""
echo -e "${GREEN}✅ Downloaded ${GREEN}${downloaded}/${#files[@]}${NC} files${NC}"
echo ""
echo -e "📖 ${BLUE}Documentation files:${NC}"
echo "   - ${TARGET_DIR}/search.md       (Semantic code discovery)"
echo "   - ${TARGET_DIR}/callers.md      (Reverse dependency lookup)"
echo "   - ${TARGET_DIR}/explore.md      (File anatomy discovery)"
echo "   - ${TARGET_DIR}/workflows.md    (Common patterns & best practices)"
echo ""
echo -e "🚀 ${BLUE}Quick start:${NC}"
echo "   knot search \"your query\""
echo "   knot explore \"src/path/to/file.ts\""
echo "   knot callers \"EntityName\""
echo ""
echo -e "📖 ${BLUE}Read the guides:${NC}"
echo "   less ${TARGET_DIR}/search.md"
echo "   less ${TARGET_DIR}/workflows.md"
