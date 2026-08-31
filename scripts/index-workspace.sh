#!/usr/bin/env bash
# Index every git repository in a workspace directory.
#
# Usage:
#   ./scripts/index-workspace.sh /path/to/org-checkout
#
# Environment:
#   KNOT_BIN          — path to knot-indexer (default: ./target/release/knot-indexer)
#   KNOT_NEO4J_PASSWORD — Neo4j password (default: knot_secret_password)
#   KNOT_CLEAN        — set to 1 to pass --clean to each indexer run

set -euo pipefail

WORKDIR="${1:?Usage: $0 /path/to/workspace}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KNOT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
KNOT_BIN="${KNOT_BIN:-$KNOT_ROOT/target/release/knot-indexer}"
KNOT_CLI="${KNOT_CLI:-$KNOT_ROOT/target/release/knot}"
NEO4J_PASSWORD="${KNOT_NEO4J_PASSWORD:-knot_secret_password}"

if [[ ! -x "$KNOT_BIN" ]]; then
  echo "error: knot-indexer not found at $KNOT_BIN (run: cargo build --release)" >&2
  exit 1
fi

indexed=0
skipped=0

for dir in "$WORKDIR"/*/; do
  [[ -d "$dir/.git" ]] || { ((skipped++)) || true; continue; }
  name="$(basename "$dir")"
  echo "=== [$((indexed + 1))] Indexing $name ==="
  if [[ "${KNOT_CLEAN:-}" == "1" ]]; then
    "$KNOT_BIN" --repo-path "$dir" --repo-name "$name" --neo4j-password "$NEO4J_PASSWORD" --clean
  else
    "$KNOT_BIN" --repo-path "$dir" --repo-name "$name" --neo4j-password "$NEO4J_PASSWORD"
  fi
  ((indexed++)) || true
done

echo ""
echo "Indexed: $indexed repo(s), skipped: $skipped non-git folder(s)"
if [[ -x "$KNOT_CLI" ]]; then
  echo ""
  "$KNOT_CLI" repos
else
  echo "(build knot CLI to see inventory: cargo build --release --bin knot)"
fi
