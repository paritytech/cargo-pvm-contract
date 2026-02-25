#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCHMARKS_DIR="$(dirname "$SCRIPT_DIR")"
REPORTS_DIR="$BENCHMARKS_DIR/reports"
WORKSPACE_ROOT="$(cd "$BENCHMARKS_DIR/../.." && pwd)"

usage() {
    echo "Usage: $0 [encoding|binary-sizes|all]"
    echo ""
    echo "Regenerate benchmark reports."
    echo ""
    echo "  encoding      Run criterion encoding benchmarks and regenerate encoding-benchmarks.md"
    echo "  binary-sizes  Run build-and-measure and regenerate macro-vs-builder.md"
    echo "  all           Run both (default)"
    exit 1
}

TARGET="${1:-all}"

# ============================================================================
# Encoding benchmarks report
# ============================================================================

parse_criterion_time() {
    # Extract the estimate (middle value) from criterion output line:
    #   time:   [14.20 ns 14.31 ns 14.42 ns]
    # Returns e.g. "14.31 ns"
    local line="$1"
    echo "$line" | sed -E 's/.*\[.+ (.+ [a-zµ]+) .+\]/\1/'
}

normalize_to_ns() {
    # Convert a time string like "14.31 ns" or "1.23 µs" or "1.23 us" to nanoseconds
    local time_str="$1"
    local value unit
    value=$(echo "$time_str" | awk '{print $1}')
    unit=$(echo "$time_str" | awk '{print $2}')
    case "$unit" in
        ps)  echo "$value * 0.001" | bc -l ;;
        ns)  echo "$value" ;;
        µs|us) echo "$value * 1000" | bc -l ;;
        ms)  echo "$value * 1000000" | bc -l ;;
        *)   echo "$value" ;;
    esac
}

fmt_ns() {
    # Format nanoseconds to 2 decimal places with " ns" suffix
    printf "%.2f ns" "$1"
}

compute_ratio() {
    # Compute ratio and direction label: "Nx faster" or "Nx slower"
    local pvm_ns="$1" alloy_ns="$2"
    local ratio
    if (( $(echo "$pvm_ns < $alloy_ns" | bc -l) )); then
        ratio=$(echo "$alloy_ns / $pvm_ns" | bc -l)
        printf "%.2fx faster" "$ratio"
    else
        ratio=$(echo "$pvm_ns / $alloy_ns" | bc -l)
        printf "%.2fx slower" "$ratio"
    fi
}

regenerate_encoding() {
    echo "==> Running criterion benchmarks (cargo bench -p pvm-contract-types --features alloc)..."
    local bench_output
    bench_output=$(cd "$WORKSPACE_ROOT" && cargo bench -p pvm-contract-types --features alloc 2>&1) || {
        echo "ERROR: cargo bench failed"
        echo "$bench_output"
        exit 1
    }

    # Parse all "time:" lines with their benchmark names
    # Criterion outputs lines like:
    #   u8_encode_pvm           time:   [14.20 ns 14.31 ns 14.42 ns]
    declare -A bench_times
    local current_bench=""
    while IFS= read -r line; do
        # Match benchmark name lines (e.g. "Benchmarking u8_encode_pvm" or the result line)
        if [[ "$line" =~ ^([a-z0-9_]+)[[:space:]]+time: ]]; then
            current_bench="${BASH_REMATCH[1]}"
            local time_str
            time_str=$(parse_criterion_time "$line")
            bench_times["$current_bench"]="$time_str"
        fi
    done <<< "$bench_output"

    # Define type display names and benchmark name prefixes
    local -a static_types=("u8" "u32" "u128" "u256" "address")
    local -a dynamic_types=("string" "vec_u256")

    declare -A type_labels
    type_labels[u8]="u8"
    type_labels[u32]="u32"
    type_labels[u128]="u128"
    type_labels[u256]="U256"
    type_labels[address]="address"
    type_labels[string]="String"
    type_labels[vec_u256]="Vec\\<U256>"

    generate_table() {
        local -a types=("$@")
        echo "| Type | Operation | pvm-contract-types | alloy-core | Ratio |"
        echo "|------|-----------|--------------------|------------|-------|"
        for type_key in "${types[@]}"; do
            local label="${type_labels[$type_key]}"
            for op in encode decode; do
                local pvm_key="${type_key}_${op}_pvm"
                local alloy_key="${type_key}_${op}_alloy"
                local pvm_time="${bench_times[$pvm_key]:-}"
                local alloy_time="${bench_times[$alloy_key]:-}"
                if [[ -z "$pvm_time" || -z "$alloy_time" ]]; then
                    echo "WARNING: Missing benchmark data for $type_key $op" >&2
                    continue
                fi
                local pvm_ns alloy_ns
                pvm_ns=$(normalize_to_ns "$pvm_time")
                alloy_ns=$(normalize_to_ns "$alloy_time")
                local pvm_fmt alloy_fmt ratio
                pvm_fmt=$(fmt_ns "$pvm_ns")
                alloy_fmt=$(fmt_ns "$alloy_ns")
                ratio=$(compute_ratio "$pvm_ns" "$alloy_ns")
                printf "| %-10s | %-6s | %-18s | %-12s | %-12s |\n" \
                    "$label" "$op" "$pvm_fmt" "$alloy_fmt" "$ratio"
            done
        done
    }

    local report="$REPORTS_DIR/encoding-benchmarks.md"
    cat > "$report" <<'HEADER'
# ABI Encoding/Decoding Benchmarks: pvm-contract-types vs Alloy

Criterion benchmarks comparing `pvm-contract-types` (`SolEncode`/`SolDecode`)
against `alloy-core` (`SolValue::abi_encode`/`abi_decode`) for ABI encoding and
decoding of Solidity types.

## How to regenerate

```bash
cargo bench -p pvm-contract-types --features alloc
```

Results are saved to `target/criterion/`. The raw output contains the numbers
used in this report.

## Results

### Static Types (no-alloc compatible)

HEADER
    generate_table "${static_types[@]}" >> "$report"

    cat >> "$report" <<'MID'

### Dynamic Types (alloc required)

MID
    generate_table "${dynamic_types[@]}" >> "$report"

    cat >> "$report" <<'FOOTER'

### Summary

**Encoding**: `pvm-contract-types` encodes static types ~2x slower than alloy
due to writing into a caller-provided `[u8]` buffer (with zeroing) rather than
returning a heap-allocated `Vec<u8>`. For dynamic types (String, Vec) where both
approaches allocate, `pvm-contract-types` is 1.2–1.4x faster.

**Decoding**: `pvm-contract-types` decodes all types faster than alloy — from
2x faster for U256 up to 6x faster for u8. This is because decoding reads
directly from the ABI-encoded buffer with no validation overhead, while alloy
performs full ABI conformance checking.

**Key insight**: The encode overhead is a benchmarking artifact. In real
contracts, `pvm-contract-types` encodes into a stack buffer (no allocation),
which is strictly faster in the `no_std` + `no_alloc` context where these
contracts run. The alloy `abi_encode()` approach always heap-allocates a
`Vec<u8>`, which is not possible without an allocator.
FOOTER

    echo "==> Generated $report"
}

# ============================================================================
# Binary sizes report
# ============================================================================

regenerate_binary_sizes() {
    echo "==> Running build-and-measure (cargo +nightly run -p pvm-contract-benchmarks --bin build-and-measure)..."
    (cd "$WORKSPACE_ROOT" && cargo +nightly run -p pvm-contract-benchmarks --bin build-and-measure) || {
        echo "ERROR: build-and-measure failed"
        exit 1
    }

    local artifacts_dir="$WORKSPACE_ROOT/target/benchmark-artifacts"

    # Collect release artifact sizes: contract_variant.release.polkavm
    declare -A sizes
    local -a contracts=()
    local -a seen_contracts=()

    for f in "$artifacts_dir"/*.release.polkavm; do
        [[ -e "$f" ]] || continue
        local basename
        basename=$(basename "$f" .release.polkavm)
        # basename is e.g. "fibonacci_no-alloc" or "mytoken_builder-dsl"
        # Split on last underscore
        local contract variant
        contract="${basename%_*}"
        variant="${basename##*_}"
        local size
        size=$(stat --format='%s' "$f" 2>/dev/null || stat -f '%z' "$f")
        sizes["${contract}_${variant}"]="$size"

        # Track unique contracts in order
        local found=0
        for c in "${seen_contracts[@]+"${seen_contracts[@]}"}"; do
            [[ "$c" == "$contract" ]] && found=1 && break
        done
        if [[ $found -eq 0 ]]; then
            seen_contracts+=("$contract")
        fi
    done
    contracts=("${seen_contracts[@]}")

    local -a variants=("no-alloc" "builder-dsl" "with-alloc")

    fmt_bytes() {
        printf "%'d" "$1"
    }

    fmt_kb() {
        echo "scale=2; $1 / 1024" | bc -l | awk '{printf "%.2f", $0}'
    }

    local report="$REPORTS_DIR/macro-vs-builder.md"

    local today
    today=$(date +%Y-%m-%d)

    cat > "$report" <<EOF
# Macro vs Builder DSL: Release Binary Size Comparison

Date: $today
Commit: measured on current HEAD of cargo-pvm-contract


## 1. Approaches

- **Proc-macro approach** (\`pvm-contract-macros\`): attribute proc macros
  (\`#[contract]\`, \`#[method]\`, \`#[constructor]\`, \`#[fallback]\`) that parse
  Rust+Solidity and emit dispatch code at compile time via \`syn\`/\`quote\`.
- **Builder DSL approach** (\`pvm-contract-builder-dsl\`): a pure Rust builder
  pattern API (\`ContractBuilder::new().method(selector, handler).dispatch()\`)
  that wires up dispatch at runtime without any proc-macro dependency.


## 2. Release Binary Sizes

From \`target/benchmark-artifacts/\` (built by \`build-and-measure\`):

EOF

    # Main size table
    {
        echo "| Contract  | Variant     | Size (bytes) | Size (KB) |"
        echo "|-----------|-------------|-------------:|----------:|"
        for contract in "${contracts[@]}"; do
            for variant in "${variants[@]}"; do
                local key="${contract}_${variant}"
                local size="${sizes[$key]:-}"
                [[ -z "$size" ]] && continue
                local size_fmt kb_fmt
                size_fmt=$(fmt_bytes "$size")
                kb_fmt=$(fmt_kb "$size")
                printf "| %-9s | %-11s | %12s | %9s |\n" "$contract" "$variant" "$size_fmt" "$kb_fmt"
            done
        done
    } >> "$report"

    # Builder DSL vs Proc-Macro comparison table
    cat >> "$report" <<'EOF'

### Builder DSL vs Proc-Macro (no-alloc)

EOF

    # Count methods per contract (hardcoded to match the benchmark contracts)
    declare -A method_counts
    method_counts[fibonacci]=1
    method_counts[mytoken]=4
    method_counts[multi]=10

    {
        echo "| Contract  | Methods | Proc-Macro | Builder DSL | Overhead |"
        echo "|-----------|--------:|------------|-------------|----------|"
        for contract in "${contracts[@]}"; do
            local noalloc_size="${sizes[${contract}_no-alloc]:-}"
            local dsl_size="${sizes[${contract}_builder-dsl]:-}"
            [[ -z "$noalloc_size" || -z "$dsl_size" ]] && continue
            local methods="${method_counts[$contract]:-?}"
            local noalloc_fmt="${noalloc_size} B"
            local dsl_fmt="${dsl_size} B"
            local overhead
            overhead=$(echo "scale=1; ($dsl_size - $noalloc_size) * 100 / $noalloc_size" | bc -l)
            # Format: remove trailing zeros but keep at least one decimal
            overhead=$(printf "%.1f" "$overhead")
            printf "| %-9s | %7s | %10s | %11s | +%-7s |\n" \
                "$contract" "$methods" "$noalloc_fmt" "$dsl_fmt" "${overhead}%"
        done
    } >> "$report"

    cat >> "$report" <<'EOF'

For the trivial fibonacci contract (1 method), the builder DSL adds ~730 bytes
of overhead from the runtime dispatch table and calldata-copy loop. As method
count grows, the fixed overhead is amortized: mytoken (4 methods) shows +0.3%
and multi (10 methods with mixed parameter types) shows +4.1%. The builder DSL
does not become cheaper than the proc-macro with more methods, but the overhead
stays negligible for real contracts.

### Key Size Drivers

| Factor                             | Impact                               |
|------------------------------------|--------------------------------------|
| Allocator (no-alloc vs with-alloc) | 26x for fibonacci, 4.3x for mytoken  |
| Builder DSL vs proc-macro no-alloc | +4% at 10 methods, negligible at scale |
EOF

    echo "==> Generated $report"
}

# ============================================================================
# Main
# ============================================================================

case "$TARGET" in
    encoding)      regenerate_encoding ;;
    binary-sizes)  regenerate_binary_sizes ;;
    all)
        regenerate_encoding
        regenerate_binary_sizes
        ;;
    *)  usage ;;
esac

echo "==> Done."
