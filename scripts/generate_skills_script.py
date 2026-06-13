#!/usr/bin/env python3
"""
Generate .knot-agent-skills.sh from docs/agent-skills/*.md.

Each markdown file is base64-encoded and embedded in the shell script as a
single-quoted string. The shell script decodes them on install.
"""
from __future__ import annotations

import base64
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SKILL_FILES = [
    "search.md",
    "callers.md",
    "explore.md",
    "deps.md",
    "repos.md",
    "workflows.md",
]

SCRIPT_HEADER = r"""#!/usr/bin/env bash
#
# Knot Agent-Skills Documentation Installer
#
# Installs the six skill documentation files used by LLM agents to learn
# the knot CLI/MCP toolchain. The markdown payloads are base64-encoded
# inside this script and decoded on install.
#
# Requires: bash >= 3.2 (uses arrays, `wait`, `printf %b`, `set -o pipefail`).
#           base64  — standard on Linux, macOS, and the Windows Git Bash MSYS
#                     environment that OpenCode users typically run.
#
# Usage:
#   ./.knot-agent-skills.sh                       # install into ./.knot-agent-skills
#   ./.knot-agent-skills.sh /path/to/install/dir  # install elsewhere
#   ./.knot-agent-skills.sh --no-register         # skip the OpenCode prompt
#   curl -fsSL https://.../.knot-agent-skills.sh | bash -s -- --no-register
#
# After the files are extracted, the script prompts to register the skills
# with OpenCode (global or project-local config). Use --no-register to skip
# the prompt (useful for non-interactive runs).

# Self-detect bash. The shebang above is the canonical entry point, but a
# user might still run `sh .knot-agent-skills.sh` (dash on Debian, ash on
# Alpine, …) which breaks the `set -o pipefail` and the `printf %b` output
# formatting below. Re-exec under bash so the rest of the script can rely
# on bash features without surprising the user.
if [ -z "${BASH_VERSION:-}" ]; then
  if command -v bash >/dev/null 2>&1; then
    exec bash "$0" "$@"
  else
    printf 'bash is required to run this installer.\n' >&2
    exit 1
  fi
fi

set -e
set -u
set -o pipefail

# --- Argument parsing ---------------------------------------------------------

NO_REGISTER=0
POSITIONAL=()
for arg in "$@"; do
  case "$arg" in
    --no-register)
      NO_REGISTER=1
      ;;
    -h|--help)
      cat <<'USAGE'
Usage: .knot-agent-skills.sh [TARGET_DIR] [--no-register]

Arguments:
  TARGET_DIR    Directory to extract the .md files into (default: .knot-agent-skills)
  --no-register Skip the OpenCode registration prompt
  -h, --help    Show this help message
USAGE
      exit 0
      ;;
    --)
      shift
      while [ $# -gt 0 ]; do
        POSITIONAL+=("$1")
        shift
      done
      ;;
    -*)
      printf 'Unknown option: %s\n' "$arg" >&2
      exit 2
      ;;
    *)
      POSITIONAL+=("$arg")
      ;;
  esac
done

TARGET_DIR="${POSITIONAL[0]:-.knot-agent-skills}"

# --- Colours (printf %b instead of echo -e) ----------------------------------

if [ -t 1 ] && command -v tput >/dev/null 2>&1 && [ -n "${TERM:-}" ] && [ "${TERM:-}" != "dumb" ]; then
  RED=$(printf '\033[0;31m')
  GREEN=$(printf '\033[0;32m')
  YELLOW=$(printf '\033[1;33m')
  BLUE=$(printf '\033[0;34m')
  CYAN=$(printf '\033[0;36m')
  BOLD=$(printf '\033[1m')
  NC=$(printf '\033[0m')
else
  RED=""
  GREEN=""
  YELLOW=""
  BLUE=""
  CYAN=""
  BOLD=""
  NC=""
fi

# --- Extraction --------------------------------------------------------------

if ! command -v base64 >/dev/null 2>&1; then
  printf '%b!%b base64 is not available on this system. Aborting.\n' "$YELLOW" "$NC" >&2
  exit 1
fi

mkdir -p "$TARGET_DIR"

printf '%b%b📦 Installing knot agent-skills documentation...%b\n' "$BLUE" "$BOLD" "$NC"
printf '   Destination: %b%s%b\n' "$GREEN" "$TARGET_DIR" "$NC"
printf '\n'

# Function: decode a base64 blob and write it to $TARGET_DIR/$filename.
# `printf %s` preserves the payload byte-for-byte (no shell interpretation).
write_file() {
  local filename="$1"
  local payload="$2"
  local outpath="$TARGET_DIR/$filename"
  local tmpfile
  tmpfile=$(mktemp)
  printf '%s' "$payload" | base64 -d > "$tmpfile"
  # Restore normal umask so the resulting file is readable. `mktemp` defaults
  # to 0600 which would make the installed skill invisible to other tools.
  chmod 0644 "$tmpfile"
  mv "$tmpfile" "$outpath"
  printf '   %b✓%b %s\n' "$GREEN" "$NC" "$filename"
}

"""

# Build the file-write block: one `write_file "name" "BASE64_BLOB"` per file.
# We use double-quoted strings because the base64 alphabet has no special
# characters for bash inside double quotes.
write_lines: list[str] = []
for fname in SKILL_FILES:
    src = REPO_ROOT / "docs" / "agent-skills" / fname
    if not src.exists():
        print(f"missing: {src}", file=sys.stderr)
        sys.exit(1)
    encoded = base64.b64encode(src.read_bytes()).decode("ascii")
    write_lines.append(f'write_file "{fname}" "{encoded}"')

write_block = "\n".join(write_lines) + "\n\n"

# We need to be careful with the heredoc syntax for the skill JSON below.
# Use placeholders that do not collide with the f-string interpolator.
SCRIPT_FOOTER = r"""
# --- Done --------------------------------------------------------------------

printf '\n'
printf '%b%b✅ Installation complete!%b\n' "$GREEN" "$BOLD" "$NC"
printf '\n'
printf '📖 %bDocumentation files:%b\n' "$BLUE" "$NC"
ls -1 "$TARGET_DIR" | sed 's/^/   - /'
printf '\n'
printf '📚 %bGet started:%b\n' "$BLUE" "$NC"
printf '   cat %s/search.md\n' "$TARGET_DIR"
printf '   knot search "your query"\n'

# --- OpenCode registration prompt --------------------------------------------

if [ "$NO_REGISTER" = "1" ]; then
  exit 0
fi

# Skip the prompt if stdin is not a TTY (e.g. piped from curl | bash).
if [ ! -t 0 ]; then
  exit 0
fi

printf '\n'
printf '%b%b🔧 OpenCode skill registration%b\n' "$CYAN" "$BOLD" "$NC"
printf 'The installed skill files can be registered with OpenCode so they\n'
printf 'appear under /skills in the agent. Choose where to register them:\n'
printf '\n'
printf '   %b1)%b Global      — patch %s\n' "$BOLD" "$NC" "$HOME/.config/opencode/opencode.json"
printf '   %b2)%b Project     — patch ./opencode.json\n' "$BOLD" "$NC"
printf '   %b3)%b Skip        — exit without registering\n'
printf '\n'

# Read a single character without waiting for Enter.
read -r -n 1 -p "Select [1/2/3]: " choice
printf '\n'

# Resolve the absolute install directory once so that the resulting
# file:// URIs in opencode.json work no matter where the user runs the
# script from later.
ABS_SKILL_DIR=$(cd "$TARGET_DIR" && pwd)

register_with_python() {
  local target="$1"
  SKILL_DIR="$ABS_SKILL_DIR" TARGET_FILE="$target" python3 - <<'PYEOF' || return 1
import json, os, re, sys

target = os.environ["TARGET_FILE"]
base = os.environ["SKILL_DIR"]

mapping = [
    ("search.md",    "knot-search",    "Use knot search for semantic code discovery across indexed repositories"),
    ("callers.md",   "knot-callers",   "Use knot callers to find reverse dependencies and perform impact analysis"),
    ("explore.md",   "knot-explore",   "Use knot explore to get a structural overview of a source file"),
    ("deps.md",      "knot-deps",      "Use knot deps to traverse the repository dependency graph"),
    ("repos.md",     "knot-repos",     "Use knot repos to list all indexed repositories and their status"),
    ("workflows.md", "knot-workflows", "Multi-step knot workflows: impact analysis, cross-repo exploration, refactoring patterns"),
]

new_skills = [
    {"name": name, "description": desc, "location": f"file://{base}/{fname}"}
    for fname, name, desc in mapping
]

# opencode.json files in the wild frequently contain `//` and `#` comments
# (the user's config is hand-edited) plus occasional trailing commas. Strip
# those before parsing so we do not refuse to edit otherwise-valid configs.
def _strip_jsonc(text: str) -> str:
    text = re.sub(r"^\s*//.*$", "", text, flags=re.MULTILINE)
    text = re.sub(r"\s+//[^\n]*", "", text)
    text = re.sub(r"^\s*#.*$", "", text, flags=re.MULTILINE)
    text = re.sub(r",(\s*[\]}])", r"\1", text)
    return text

try:
    with open(target, "r", encoding="utf-8") as f:
        raw = f.read()
    if not raw.strip():
        cfg = {"$schema": "https://opencode.ai/config.json"}
    else:
        cfg = json.loads(_strip_jsonc(raw))
except FileNotFoundError:
    cfg = {"$schema": "https://opencode.ai/config.json"}
except json.JSONDecodeError as exc:
    print(f"opencode.json is not valid JSON/JSONC: {exc}", file=sys.stderr)
    sys.exit(2)

if not isinstance(cfg, dict):
    cfg = {}

existing = {
    s.get("name"): s
    for s in cfg.get("skills", [])
    if isinstance(s, dict) and s.get("name")
}
for s in new_skills:
    existing[s["name"]] = s

cfg["skills"] = list(existing.values())

with open(target, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2)
    f.write("\n")
PYEOF
}

register_with_jq() {
  local target="$1"
  local tmp_skills tmp_cfg
  tmp_skills=$(mktemp)
  tmp_cfg=$(mktemp)

  cat > "$tmp_skills" <<EOFJSON
[
  { "name": "knot-search",    "description": "Use knot search for semantic code discovery across indexed repositories", "location": "file://${ABS_SKILL_DIR}/search.md" },
  { "name": "knot-callers",   "description": "Use knot callers to find reverse dependencies and perform impact analysis", "location": "file://${ABS_SKILL_DIR}/callers.md" },
  { "name": "knot-explore",   "description": "Use knot explore to get a structural overview of a source file", "location": "file://${ABS_SKILL_DIR}/explore.md" },
  { "name": "knot-deps",      "description": "Use knot deps to traverse the repository dependency graph", "location": "file://${ABS_SKILL_DIR}/deps.md" },
  { "name": "knot-repos",     "description": "Use knot repos to list all indexed repositories and their status", "location": "file://${ABS_SKILL_DIR}/repos.md" },
  { "name": "knot-workflows", "description": "Multi-step knot workflows: impact analysis, cross-repo exploration, refactoring patterns", "location": "file://${ABS_SKILL_DIR}/workflows.md" }
]
EOFJSON

  if [ ! -f "$target" ]; then
    printf '{\n  "$schema": "https://opencode.ai/config.json",\n  "skills": []\n}\n' > "$target"
  fi

  # Replace any existing knot-* entries with the new set, preserving others.
  jq --slurpfile new "$tmp_skills" '
    .skills = ([.skills // [] | .[] | select((.name // "") | startswith("knot-") | not)]) + $new[0]
  ' "$target" > "$tmp_cfg"
  mv "$tmp_cfg" "$target"
  rm -f "$tmp_skills"
}

register_to() {
  local target="$1"
  if [ -z "$target" ]; then
    return 1
  fi

  mkdir -p "$(dirname "$target")"

  if command -v python3 >/dev/null 2>&1; then
    if register_with_python "$target"; then
      printf '   %b✓%b Registered in %s\n' "$GREEN" "$NC" "$target"
      return 0
    fi
  fi

  if command -v jq >/dev/null 2>&1; then
    register_with_jq "$target"
    printf '   %b✓%b Registered in %s\n' "$GREEN" "$NC" "$target"
    return 0
  fi

  printf '   %b!%b Could not register automatically: neither python3 nor jq is installed.\n' "$YELLOW" "$NC"
  printf '   Please add the following to %s:\n' "$target"
  printf '\n'
  cat <<MANUAL
  "skills": [
    { "name": "knot-search",    "description": "Use knot search for semantic code discovery across indexed repositories", "location": "file://${ABS_SKILL_DIR}/search.md" },
    { "name": "knot-callers",   "description": "Use knot callers to find reverse dependencies and perform impact analysis", "location": "file://${ABS_SKILL_DIR}/callers.md" },
    { "name": "knot-explore",   "description": "Use knot explore to get a structural overview of a source file", "location": "file://${ABS_SKILL_DIR}/explore.md" },
    { "name": "knot-deps",      "description": "Use knot deps to traverse the repository dependency graph", "location": "file://${ABS_SKILL_DIR}/deps.md" },
    { "name": "knot-repos",     "description": "Use knot repos to list all indexed repositories and their status", "location": "file://${ABS_SKILL_DIR}/repos.md" },
    { "name": "knot-workflows", "description": "Multi-step knot workflows: impact analysis, cross-repo exploration, refactoring patterns", "location": "file://${ABS_SKILL_DIR}/workflows.md" }
  ]
MANUAL
}

case "$choice" in
  1)
    register_to "$HOME/.config/opencode/opencode.json"
    ;;
  2)
    register_to "./opencode.json"
    ;;
  3|"")
    printf '   Skipped.\n'
    ;;
  *)
    printf '   %b!%b Unknown choice; skipping.\n' "$YELLOW" "$NC" >&2
    ;;
esac

printf '\n'
"""


def main() -> None:
    out = REPO_ROOT / ".knot-agent-skills.sh"
    parts = [SCRIPT_HEADER, write_block, SCRIPT_FOOTER]
    out.write_text("".join(parts))
    out.chmod(0o755)
    print(f"wrote {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
