#!/usr/bin/env bash
set -euo pipefail

export PATH="/home/pubuntu/.cargo/bin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "Building release qf..."
RUSTC_WRAPPER="" cargo build --release --offline

QF_BIN="$ROOT_DIR/target/release/qf"
DATA_FILE="/tmp/qf_bench_20mb.json"
trap 'rm -f "$DATA_FILE"' EXIT

echo "Generating 300k-object JSON test file at $DATA_FILE..."
python3 - <<'PY' "$DATA_FILE"
import sys

path = sys.argv[1]
n = 300_000
with open(path, "w") as f:
    f.write("[\n")
    for i in range(n):
        comma = "," if i + 1 < n else ""
        f.write(f'{{"id":{i},"v":{i % 1000},"name":"item_{i}","active":{"true" if i % 2 == 0 else "false"}}}{comma}\n')
    f.write("]\n")
PY

ls -lh "$DATA_FILE"

QUERIES=(
    'map(.v)|add'
    '[.[]|select(.id%2==0)]|length'
    'reduce .[] as $x (0; .+$x.id)'
)

run_bench() {
    local label="$1"
    local bin="$2"
    local query="$3"
    local out
    local stats
    stats=$( { /usr/bin/time -f "elapsed=%es max_rss=%MKB" "$bin" "$query" "$DATA_FILE" > /tmp/qf_bench_out.tmp; } 2>&1 )
    out=$(head -n 1 /tmp/qf_bench_out.tmp | tr -d '[:space:]')
    rm -f /tmp/qf_bench_out.tmp
    printf "  %-4s | %-34s | out=%-12s | %s\n" "$label" "$query" "$out" "$stats"
}

echo ""
echo "=== Benchmark Results ==="
for q in "${QUERIES[@]}"; do
    run_bench "qf" "$QF_BIN" "$q"
    if command -v jq >/dev/null 2>&1; then
        run_bench "jq" "jq" "$q"
    fi
done
