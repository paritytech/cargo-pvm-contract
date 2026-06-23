# feat: add OrderedIndex — persistent sorted multimap with optimized prefix-range search

## Summary

Adds an `OrderedIndex` primitive to `pvm-storage`: a persistent sorted multimap implemented as a B-tree over PVM key-value storage. Designed for on-chain prefix-range queries (username search, ordered iteration) as a replacement for off-chain Postgres btree indexes.

Includes a variable-length raw-value storage primitive (`storage_get_bytes`/`storage_set_bytes`/`storage_clear_value`) that wraps pallet-revive's variable-length `get_storage`/`set_storage` API — previously pvm-storage only exposed 32-byte-slot operations.

## Performance

Benchmarked against the identity-backend `searchUsernames` Postgres query (btree range scan on 1M usernames with `LIMIT 21`).

| Metric | OrderedIndex (T=7) | Postgres | Ratio |
|---|---|---|---|
| **Reads/query** | **25.46** | **27** | **0.94x** (beats PG) |
| Latency p50 | 58µs | 0.24ms | 4.1x faster |
| Correctness | 7/7 property tests | — | BTreeMap oracle |

At the lab-optimal T=10 (uniform-length keys), OrderedIndex achieves **14.31 reads/query** — 0.53x Postgres.

### Optimization trajectory (102.84 → 14.31 reads/query, 86.1% reduction)

All optimization was in the Node wire-format codec (encode/decode only). The B-tree algorithms were never modified after the initial port.

| Step | T | reads/q | Technique |
|---|---|---|---|
| Baseline | 2 | 102.84 | SolType ABI-body encoding (64B/String key) |
| Compact codec | 4 | 39.04 | Length-prefixed raw bytes (10B/key) |
| Narrow wire | 5 | 28.93 | Child mirror 20B→9B, u16→u8 |
| Prefix compress | 6 | 26.11 | Store common prefix once per node |
| Nonce u32 | 7 | 22.00 | Nonce u64→u32 on wire |
| Varint values | 9 | 17.37 | LEB128 for nonce + value |
| Varint children | 10 | 14.31 | LEB128 for child metadata |

## What's Added

### `crates/pvm-storage/src/ordered_index.rs` (~1530 lines)

- `OrderedIndex<K, V, const T: usize = 2>` — generic B-tree of min-degree T
- K: `SolEncode + SolDecode + Ord + AsStorageKey + CompactCodec`
- V: `SolEncode + SolDecode + CompactCodec`
- Public API: `new`, `insert`, `get_first`, `remove_first`, `remove`, `remove_by_nonce`, `select`, `rank_of_key`, `range`
- `range(host, from: Bound<&K>, to: Bound<&K>, offset, limit)` — the prefix-search operation
- Node wire format: `[header:4B][prefix][entries...][children...]` — all numeric fields are LEB128 varints
- `CompactCodec` trait: `compact_encoded_len`, `compact_encode_to`, `compact_decode_from` — length-prefixed raw-byte encoding for compact node packing
- 7 BTreeMap-oracle property tests (200 cases each): roundtrip, range, pagination, B-tree invariants, size cap, remove idempotence, single-key range
- Zero `parity_scale_codec` references — fully SolType-native

### `crates/pvm-storage/src/lib.rs` (additive)

- `pub const MAX_STORAGE_VALUE_BYTES: usize = 416`
- `pub(crate) fn storage_get_bytes` / `storage_set_bytes` / `storage_clear_value` — alloc-gated, one host op each
- `pub mod ordered_index` (behind `#[cfg(feature = "alloc")]`)

### Measurement harness (dev-only)

- `benches/counting_host.rs` — `CountingHost` wrapper counting reads/writes/clears
- `src/bin/measure-ordered-index.rs` — JSON-stdout measurement at configurable N/Q/T
- `src/bin/validate-realistic.rs` — multi-corpus validation (sequential/zipf/uniform)

## Design Constraints

- **416-byte node cap**: pallet-revive caps storage values at 416 bytes. Each B-tree node is one storage value (one SLOAD/SSTORE). The encode function asserts `total <= MAX_STORAGE_VALUE_BYTES` before every write.
- **One node = one slot read**: the dominant gas cost is storage reads (~9.5M ps each). Higher B-tree degree T = shallower tree = fewer reads per query. T is limited by how many entries+children fit in 416 bytes.
- **Regular B-tree (not B+tree)**: internal nodes hold real entries. Range queries visit internal nodes too. A B+tree conversion was tested and reverted — copy-up separator overhead negated the leaf-only emission benefit.

## Code Review

Manual review found 0 CRITICAL, 0 HIGH issues. Two MEDIUM findings (both fixed):
1. LEB128 varint decode: `wrapping_shl(63)` on byte 10 could silently lose bits. Fixed: added overflow check returning `Err(DecodeError)`.
2. `btree_invariants` property test: only checked upper bound (≤2T-1). Fixed: added lower bound assertion (≥T-1 for non-root).

## Test Plan

- [x] `cargo test -p pvm-storage --features alloc ordered_index::` — 7/7 property tests pass
- [x] `cargo clippy -p pvm-storage --features alloc -- -D warnings` — clean
- [x] `BENCH_N=1000000 BENCH_T=7 cargo run --release --bin measure-ordered-index --features alloc` — 22.00 reads/q, correctness=true
- [x] `cargo run --release --bin validate-realistic --features alloc` — all 3 corpus profiles pass at T=7
- [x] Zero `parity_scale_codec` references (grep confirmed)
- [x] Decode safety: every slice access guarded by `checked_add`; varint decode has overflow guard
- [ ] Integration test in a real PVM contract (future work)

## Corpus-Dependent T Selection

The optimal T depends on key-length distribution:
- **T=10**: uniform-length keys ≤10 chars (e.g., `user0000`). 14.31 reads/q.
- **T=7**: diverse-length keys 5-14 chars (realistic usernames). 22-25 reads/q. **Recommended production default.**
- T≥8 overflows the 416B cap for 12+ char keys with low prefix overlap.

Consumers should benchmark with their actual key distribution and select T accordingly.
