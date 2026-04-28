#!/usr/bin/env bash
# Reset the Plexus dev database — drops and recreates, letting the server
# load schema.sql on next start.
#
# Usage: ./plexus-server/scripts/reset-db.sh [db_name]
#
# Requires: psql, createdb, dropdb in PATH, plus a running PostgreSQL
# instance with credentials configured via $PGHOST/$PGUSER/$PGPASSWORD
# or a local peer-auth setup.
set -euo pipefail

DB_NAME="${1:-plexus}"

echo "Resetting database: $DB_NAME"
dropdb --if-exists "$DB_NAME"
createdb "$DB_NAME"
psql "$DB_NAME" -c "CREATE EXTENSION IF NOT EXISTS pgcrypto;"
echo "Done. Start plexus-server to load schema.sql."
