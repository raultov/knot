#!/bin/bash
#
# Knot Agent-Skills Documentation Installer
# 
# This script extracts agent-skills documentation files.
# Usage: bash .knot-agent-skills.sh [target-directory]
#
# You can also download and run in one command:
#   curl -fsSL https://raw.githubusercontent.com/user/knot/master/.knot-agent-skills.sh | bash
#

set -e

TARGET_DIR="${1:-.knot-agent-skills}"

# Create target directory
mkdir -p "$TARGET_DIR"

# Function to decode base64 and write file
write_file() {
  local filename="$1"
  local base64_content="$2"
  echo "$base64_content" | base64 -d > "$TARGET_DIR/$filename"
}

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}📦 Installing knot agent-skills documentation...${NC}"
echo -e "   Destination: ${GREEN}${TARGET_DIR}${NC}\n"

# Extract search.md
write_file "search.md" "IyBLbm90IFNlYXJjaDogU2VtYW50aWMgQ29kZSBEaXNjb3Zlcnk==" && echo -e "${GREEN}✓${NC} search.md" &
# Extract callers.md  
write_file "callers.md" "IyBLbm90IENhbGxlcnM6IFJldmVyc2UgRGVwZW5kZW5jeSBMb29rdXA=" && echo -e "${GREEN}✓${NC} callers.md" &
# Extract explore.md
write_file "explore.md" "IyBLbm90IEV4cGxvcmU6IEZpbGUgQW5hdG9teSBEaXNjb3Zlcnk=" && echo -e "${GREEN}✓${NC} explore.md" &
# Extract workflows.md
write_file "workflows.md" "IyBLbm90IFdvcmtmbG93czogUGF0dGVybnMgYW5kIEJlc3QgUHJhY3RpY2Vz" && echo -e "${GREEN}✓${NC} workflows.md" &

wait

echo ""
echo -e "${GREEN}✅ Installation complete!${NC}"
echo ""
echo -e "📖 ${BLUE}Documentation files:${NC}"
ls -1 "$TARGET_DIR" | sed 's/^/   - /'
echo ""
echo -e "📚 ${BLUE}Get started:${NC}"
echo "   cat ${TARGET_DIR}/search.md"
echo "   knot search \"your query\""
