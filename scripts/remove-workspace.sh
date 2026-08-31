#!/usr/bin/env bash
# Remove every git repository clone under a workspace directory.
#
# Deletes only immediate child folders that contain a .git directory.
# Does NOT remove the workspace parent folder itself.
# Does NOT remove Knot index data in Neo4j/Qdrant (see note below).
#
# Usage:
#   ./scripts/remove-workspace.sh /path/to/org-checkout           # prompt before delete
#   ./scripts/remove-workspace.sh /path/to/org-checkout --yes    # no prompt
#   ./scripts/remove-workspace.sh /path/to/org-checkout --dry-run # list only
#
# To also wipe indexed data for those repos:
#   KNOT_PURGE_INDEX=1 ./scripts/remove-workspace.sh ~/code/Synthapse --yes
#   # or reset databases entirely: docker compose down -v  (in knot/)

set -euo pipefail

WORKDIR=""
ASSUME_YES=0
DRY_RUN=0

usage() {
  echo "Usage: $0 /path/to/workspace [--yes] [--dry-run]" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --yes) ASSUME_YES=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage
      ;;
    *)
      if [[ -z "$WORKDIR" ]]; then
        WORKDIR="$1"
      else
        echo "error: unexpected argument: $1" >&2
        usage
      fi
      shift
      ;;
  esac
done

[[ -n "$WORKDIR" ]] || usage

WORKDIR="$(cd "$WORKDIR" 2>/dev/null && pwd)" || {
  echo "error: workspace not found: $WORKDIR" >&2
  exit 1
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KNOT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
KNOT_BIN="${KNOT_BIN:-$KNOT_ROOT/target/release/knot-indexer}"
NEO4J_PASSWORD="${KNOT_NEO4J_PASSWORD:-knot_secret_password}"

repos=()
for dir in "$WORKDIR"/*/; do
  [[ -d "$dir/.git" ]] || continue
  repos+=("$(basename "$dir")")
done

if [[ ${#repos[@]} -eq 0 ]]; then
  echo "No git repositories found under $WORKDIR"
  exit 0
fi

echo "Repositories to remove (${#repos[@]}):"
for name in "${repos[@]}"; do
  echo "  - $WORKDIR/$name"
done
echo ""

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "Dry run — nothing deleted."
  exit 0
fi

if [[ "$ASSUME_YES" -ne 1 ]]; then
  read -r -p "Delete these directories? [y/N] " reply
  if [[ ! "$reply" =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
  fi
fi

removed=0
for name in "${repos[@]}"; do
  path="$WORKDIR/$name"
  if [[ "${KNOT_PURGE_INDEX:-}" == "1" ]] && [[ -x "$KNOT_BIN" ]]; then
    echo "=== Purging Knot index for $name ==="
    "$KNOT_BIN" --repo-path "$path" --repo-name "$name" --neo4j-password "$NEO4J_PASSWORD" --clean 2>/dev/null || \
      echo "warning: could not purge index for $name (is Docker/Neo4j running?)" >&2
  fi
  echo "=== Removing $path ==="
  rm -rf "$path"
  ((removed++)) || true
done

echo ""
echo "Removed $removed repository clone(s) from $WORKDIR"
if [[ "${KNOT_PURGE_INDEX:-}" != "1" ]]; then
  echo "Note: Knot index entries may still exist in Neo4j/Qdrant."
  echo "      Re-run with KNOT_PURGE_INDEX=1 to purge per repo, or: docker compose down -v"
fi
