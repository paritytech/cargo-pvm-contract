#!/usr/bin/env bash
# Postgres benchmark BAR for the OrderedIndex optimization.
# Runs the REAL identity-backend username prefix-search query (the exact
# searchUsernames SQL from username-prefix-match.store.ts) against a faithful
# schema + the exact composite index, seeded with N deterministic usernames.
#
# Emits one JSON line to stdout (diagnostics to stderr):
#   {n, pg_page_reads_per_query, pg_shared_hit_per_query, pg_shared_read_per_query,
#    pg_exec_ms_p50, pg_exec_ms_p99, pg_planner_cost_total, pg_index_size_bytes, ...}
#
# Usage: pg-bar.sh [N]            (default N=10000; needs PG_CONTAINER running)
#        PG_CONTAINER=name pg-bar.sh
set -euo pipefail
N="${1:-10000}"
CONTAINER="${PG_CONTAINER:-oi-pgbar}"
PSQL=(docker exec -i "$CONTAINER" psql -U postgres -d bar -At)
PREFIXES=("user.000" "user.00" "user.0" "use")   # ~10 / ~100 / ~1000 / ~all matches

# the exact searchUsernames query (no cursor, no liteOnly), values inlined
query_sql() {
  cat <<SQL
SELECT *, (lower(coalesce(full_username, username || '.' || digits))) COLLATE "C" AS search_key
FROM individuality_usernames
WHERE network = 'westend2'
  AND (lower(coalesce(full_username, username || '.' || digits))) COLLATE "C" >= '$1'
  AND (lower(coalesce(full_username, username || '.' || digits))) COLLATE "C" <  '$2'
ORDER BY (lower(coalesce(full_username, username || '.' || digits))) COLLATE "C", username ASC, digits::integer ASC
LIMIT 21
SQL
}

# store's nextPrefixBound: increment the last character code point by one
nextbound() { python3 -c 'import sys; s=sys.argv[1]; print(s[:-1]+chr(ord(s[-1])+1))' "$1"; }

echo "[pg-bar] seeding N=$N into $CONTAINER" >&2
"${PSQL[@]}" -v ON_ERROR_STOP=1 <<SQL
DROP TABLE IF EXISTS individuality_usernames;
CREATE TABLE individuality_usernames (
  username text NOT NULL, full_username text, reserved_username text,
  digits varchar(10) NOT NULL,
  network text NOT NULL CHECK (network IN ('westend2','paseo','polkadot')),
  candidate_account_id text NOT NULL, candidate_signature text NOT NULL,
  ring_vrf_key text NOT NULL, proof_of_ownership text NOT NULL,
  consumer_registration_signature text NOT NULL, identifier_key text NOT NULL,
  candidate_signature_dotns text, signed_at timestamptz,
  status text NOT NULL DEFAULT 'RESERVED', ah_status text NOT NULL DEFAULT 'PENDING',
  source text NOT NULL DEFAULT 'INTERNAL', on_chain_data jsonb, ah_on_chain_data jsonb,
  retry_at timestamptz, retry_count integer NOT NULL DEFAULT 0,
  ah_retry_at timestamptz, ah_retry_count integer NOT NULL DEFAULT 0,
  trace_id text, span_id text, created_at timestamptz NOT NULL DEFAULT now(), updated_at timestamptz
);
CREATE INDEX individuality_username_search_key_idx ON individuality_usernames (
  network, (lower(coalesce(full_username, username || '.' || digits))) COLLATE "C"
);
INSERT INTO individuality_usernames (
  username, digits, network, candidate_account_id, candidate_signature,
  ring_vrf_key, proof_of_ownership, consumer_registration_signature, identifier_key
)
SELECT 'user', lpad(g::text, 4, '0'), 'westend2',
       '0x' || repeat('a', 64), '0x' || repeat('b', 130), '0x' || repeat('c', 64),
       '0x' || repeat('d', 130), '0x' || repeat('e', 130), '0x' || repeat('f', 64)
FROM generate_series(0, $N - 1) AS g;
ANALYZE individuality_usernames;
SQL
echo "[pg-bar] seed done; measuring" >&2

# one FORMAT-JSON plan per prefix (page-reads + cost are deterministic); Q timing samples each
Q=50
TIMINGS=$(mktemp)
declare -A PLAN_JSON
for p in "${PREFIXES[@]}"; do
  ub="$(nextbound "$p")"
  q="$(query_sql "$p" "$ub")"
  PLAN_JSON["$p"]="$("${PSQL[@]}" -c "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) $q")"
  echo "[pg-bar] prefix='$p' matches→${PLAN_JSON[$p]:0:0}captured" >&2
  for _ in $(seq 1 $Q); do
    ET="$("${PSQL[@]}" -c "EXPLAIN (ANALYZE) $q" | grep -i 'Execution Time' | sed -E 's/.*Execution Time: ([0-9.]+) ms.*/\1/' || true)"
    [ -n "$ET" ] && echo "$ET" >> "$TIMINGS"
  done
done

MEDIAN_PREFIX="user.00"
INDEX_SIZE="$("${PSQL[@]}" -c "SELECT pg_relation_size('individuality_username_search_key_idx')")"
TABLE_SIZE="$("${PSQL[@]}" -c "SELECT pg_total_relation_size('individuality_usernames')")"

python3 - "${PLAN_JSON[$MEDIAN_PREFIX]}" "$TIMINGS" "$INDEX_SIZE" "$TABLE_SIZE" "$N" "$MEDIAN_PREFIX" <<'PY'
import json, sys, statistics
plan_raw, timings_path, idx_sz, tbl_sz, n, prefix = sys.argv[1:7]
root = json.loads(plan_raw)[0]["Plan"]
def walk(node, acc):
    acc[0] += node.get("Shared Hit Blocks", 0)
    acc[1] += node.get("Shared Read Blocks", 0)
    for c in node.get("Plans", []) or []:
        walk(c, acc)
buf = [0, 0]; walk(root, buf)
with open(timings_path) as f:
    ts = sorted(float(x) for x in f if x.strip())
p50 = statistics.median(ts) if ts else None
p99 = ts[int(len(ts) * 0.99)] if ts else None
print(json.dumps({
    "n": int(n),
    "prefix_tested": prefix,
    "pg_page_reads_per_query": buf[0] + buf[1],
    "pg_shared_hit_per_query": buf[0],
    "pg_shared_read_per_query": buf[1],
    "pg_exec_ms_p50": round(p50, 4) if p50 else None,
    "pg_exec_ms_p99": round(p99, 4) if p99 else None,
    "pg_planner_cost_total": root.get("Total Cost"),
    "pg_actual_rows": root.get("Actual Rows"),
    "pg_node_type": root.get("Node Type"),
    "pg_index_size_bytes": int(idx_sz),
    "pg_table_total_size_bytes": int(tbl_sz),
    "pg_timings_count": len(ts),
}))
PY
