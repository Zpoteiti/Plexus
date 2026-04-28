#!/usr/bin/env bash
# refresh-notebooklm.sh — Sync Plexus docs to NotebookLM
# Run this whenever docs have changed significantly (or via /wrap-up).
# Usage: ./refresh-notebooklm.sh

set -euo pipefail

NOTEBOOK_ID="e1f6a0fa-e515-4829-8f03-4d64fa1f8d4a"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLEXUS_DIR="$(cd "$(dirname "$0")" && pwd)"

# Core docs — always synced
SOURCES=(
  "$REPO_ROOT/CLAUDE.md"
  "$PLEXUS_DIR/README.md"
  "$PLEXUS_DIR/plexus-server/README.md"
  "$PLEXUS_DIR/plexus-server/docs/DECISIONS.md"
  "$PLEXUS_DIR/plexus-server/docs/API.md"
  "$PLEXUS_DIR/plexus-server/docs/SCHEMA.md"
  "$PLEXUS_DIR/plexus-server/docs/SECURITY.md"
  "$PLEXUS_DIR/plexus-server/docs/DEPLOYMENT.md"
  "$PLEXUS_DIR/plexus-client/README.md"
  "$PLEXUS_DIR/plexus-client/docs/DECISIONS.md"
  "$PLEXUS_DIR/plexus-client/docs/TOOLS.md"
  "$PLEXUS_DIR/plexus-client/docs/SECURITY.md"
  "$PLEXUS_DIR/plexus-client/docs/DEPLOYMENT.md"
  "$PLEXUS_DIR/plexus-gateway/README.md"
  "$PLEXUS_DIR/plexus-gateway/docs/DECISIONS.md"
  "$PLEXUS_DIR/plexus-gateway/docs/PROTOCOL.md"
  "$PLEXUS_DIR/plexus-gateway/docs/DEPLOYMENT.md"
  "$PLEXUS_DIR/plexus-frontend/README.md"
  "$PLEXUS_DIR/plexus-frontend/docs/DECISIONS.md"
  "$PLEXUS_DIR/plexus-frontend/docs/DEPLOYMENT.md"
  "$PLEXUS_DIR/docs/superpowers/specs/2026-04-08-m1-common-client-design.md"
  "$PLEXUS_DIR/docs/superpowers/specs/2026-04-09-m2-server-design.md"
  "$PLEXUS_DIR/docs/superpowers/specs/2026-04-10-m3-gateway-frontend-design.md"
)

# Issue logs — only synced when they contain actual entries (lines starting with "- [")
ISSUE_FILES=(
  "$PLEXUS_DIR/plexus-server/docs/ISSUE.md"
  "$PLEXUS_DIR/plexus-client/docs/ISSUE.md"
  "$PLEXUS_DIR/plexus-gateway/docs/ISSUE.md"
  "$PLEXUS_DIR/plexus-frontend/docs/ISSUE.md"
)

echo "=== Plexus NotebookLM Refresh ==="
echo "Notebook: $NOTEBOOK_ID"
echo ""

# Step 1: Delete all existing sources
echo "--- Clearing old sources ---"
EXISTING=$(notebooklm source list --json --notebook "$NOTEBOOK_ID" 2>/dev/null | python3 -c "
import sys, json
data = json.load(sys.stdin)
for s in data.get('sources', []):
    print(s['id'])
" 2>/dev/null || true)

if [ -n "$EXISTING" ]; then
  while IFS= read -r source_id; do
    echo "  Deleting $source_id..."
    notebooklm source delete "$source_id" --notebook "$NOTEBOOK_ID" --yes 2>/dev/null || true
  done <<< "$EXISTING"
else
  echo "  No existing sources to remove."
fi

echo ""

# Step 2: Add core docs
echo "--- Adding core docs ---"
ADDED=0
SKIPPED=0

for filepath in "${SOURCES[@]}"; do
  if [ -f "$filepath" ]; then
    filename=$(basename "$filepath")
    echo "  Adding $filename..."
    notebooklm source add "$filepath" --notebook "$NOTEBOOK_ID" 2>/dev/null && ADDED=$((ADDED+1)) || {
      echo "  WARNING: Failed to add $filename, skipping."
      SKIPPED=$((SKIPPED+1))
    }
  else
    echo "  SKIP (not found): $filepath"
    SKIPPED=$((SKIPPED+1))
  fi
done

echo ""

# Step 3: Add issue logs only if they have content
echo "--- Adding issue logs (non-empty only) ---"
for filepath in "${ISSUE_FILES[@]}"; do
  filename=$(basename "$filepath")
  dirpart=$(basename "$(dirname "$(dirname "$filepath")")")
  label="$dirpart/$filename"
  if [ -f "$filepath" ] && grep -q "^\- \[" "$filepath" 2>/dev/null; then
    echo "  Adding $label..."
    notebooklm source add "$filepath" --notebook "$NOTEBOOK_ID" 2>/dev/null && ADDED=$((ADDED+1)) || {
      echo "  WARNING: Failed to add $label, skipping."
      SKIPPED=$((SKIPPED+1))
    }
  else
    echo "  SKIP (no issues yet): $label"
  fi
done

echo ""
echo "=== Done: $ADDED sources added, $SKIPPED skipped ==="
echo "Sources are now processing. Give it 1-2 minutes before querying."
echo "Check status: notebooklm source list --notebook $NOTEBOOK_ID"
