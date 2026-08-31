#!/usr/bin/env bash
# Clone every repository in a GitHub organisation into a workspace directory.
#
# Usage:
#   ./scripts/clone-workspace.sh Synthapse
#   ./scripts/clone-workspace.sh Synthapse ~/code/Synthapse
#   ./scripts/clone-workspace.sh Synthapse ~/code/Synthapse --no-forks
#
# Requires: gh auth login

set -euo pipefail

ORG="${1:?Usage: $0 <github-org> [workspace-dir] [--no-forks]}"
WORKDIR="${HOME}/code/${ORG}"
NO_FORKS=0

shift || true
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-forks) NO_FORKS=1; shift ;;
    -*)
      echo "error: unknown option: $1" >&2
      exit 1
      ;;
    *)
      WORKDIR="$1"
      shift
      ;;
  esac
done

mkdir -p "$WORKDIR"
WORKDIR="$(cd "$WORKDIR" && pwd)"

if ! command -v gh >/dev/null 2>&1; then
  echo "error: GitHub CLI (gh) is required" >&2
  exit 1
fi

echo "Organisation: $ORG"
echo "Workspace:    $WORKDIR"
echo ""

if [[ "$NO_FORKS" -eq 1 ]]; then
  mapfile -t repos < <(gh repo list "$ORG" --limit 1000 --json name,isFork -q '.[] | select(.isFork==false) | .name')
else
  mapfile -t repos < <(gh repo list "$ORG" --limit 1000 --json name -q '.[].name')
fi

cloned=0
skipped=0

for repo in "${repos[@]}"; do
  dest="$WORKDIR/$repo"
  if [[ -d "$dest/.git" ]]; then
    echo "Already cloned: $repo"
    ((skipped++)) || true
    continue
  fi
  echo "Cloning $repo..."
  gh repo clone "$ORG/$repo" "$dest"
  ((cloned++)) || true
done

echo ""
echo "Done: cloned $cloned, skipped $skipped (already present)"
