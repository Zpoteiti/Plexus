#!/usr/bin/env bash
set -euo pipefail

container_name="${PLEXUS_TEST_POSTGRES_CONTAINER:-plexus}"
image="${PLEXUS_TEST_POSTGRES_IMAGE:-pgvector/pgvector:pg18}"
db_name="${PLEXUS_TEST_POSTGRES_DB:-plexus}"
db_user="${PLEXUS_TEST_POSTGRES_USER:-plexus}"
db_password="${PLEXUS_TEST_POSTGRES_PASSWORD:-plexus}"
database_url="postgres://${db_user}:${db_password}@127.0.0.1:5432/${db_name}"

echo "Existing Docker containers:"
docker ps -a

container_ids="$(docker ps -aq)"
if [[ -n "${container_ids}" ]]; then
    echo "Removing all existing Docker containers..."
    # Intentionally unquoted: docker rm expects separate container ids.
    docker rm -f ${container_ids}
fi

echo "Starting PostgreSQL 18 container '${container_name}'..."
docker run -d \
    --name "${container_name}" \
    --network host \
    -e POSTGRES_USER="${db_user}" \
    -e POSTGRES_PASSWORD="${db_password}" \
    -e POSTGRES_DB="${db_name}" \
    "${image}"

echo "Waiting for PostgreSQL readiness..."
for _ in {1..60}; do
    if docker exec -e PGPASSWORD="${db_password}" "${container_name}" \
        pg_isready -h 127.0.0.1 -U "${db_user}" -d "${db_name}" >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

docker exec -e PGPASSWORD="${db_password}" "${container_name}" \
    pg_isready -h 127.0.0.1 -U "${db_user}" -d "${db_name}"

echo "Running workspace tests against ${database_url}..."
PLEXUS_TEST_DATABASE_URL="${database_url}" cargo test --workspace

echo "Databases matching plexus% after tests:"
docker exec -e PGPASSWORD="${db_password}" "${container_name}" \
    psql -h 127.0.0.1 -U "${db_user}" -d "${db_name}" \
    -c "select datname from pg_database where datname like 'plexus%' order by datname;"

echo "Public tables in persistent database '${db_name}' after tests:"
docker exec -e PGPASSWORD="${db_password}" "${container_name}" \
    psql -h 127.0.0.1 -U "${db_user}" -d "${db_name}" \
    -c "select schemaname, tablename from pg_tables where schemaname = 'public' order by tablename;"
