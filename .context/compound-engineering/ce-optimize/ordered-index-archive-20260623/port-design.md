# OrderedIndex SolType Port — Design Spec

> Status: **authoritative** for the port (Phase A2/A3). Read before editing.
> Branch: `optimize/ordered-index` (based on `main`, NOT on `charles/cdm-integration`).

## Goal

Port the `OrderedIndex` persistent sorted multimap from Charles Hetterich's
SCALE-based `pvm_contract` crate (commit `ba2656d2`, branch
`charles/cdm-integration`) INTO mainline `pvm-storage` (SolType /
`StorageEncode`+`StorageDecode`). SCALE is legacy; the ported version must have
zero `parity_scale_codec` dependency. The ported index is then the baseline for
the ce-optimize loop that optimizes slot-reads per prefix-range query against a
Postgres benchmark bar.

## Why not branch from `ba2656d2`

`charles/cdm-integration` is a 43-commit divergence that **DELETED** `pvm-storage`
and `pvm-contract-types` and replaced them with a SCALE-based `pvm_contract`
crate (−70351 / +7858 lines vs main). Branching from it would inherit a
dead-end storage stack. `main` is the SolType future; the port starts there and
re-authors `OrderedIndex` against `pvm-storage`, using Charles's file only as an
algorithm reference (extracted to `/tmp/opencode/ordered-index/ordered_index.rs`).

## The central tension (the engineering problem)

- pallet-revive allows **≤416 bytes per storage key = one SLOAD/SSTORE**.
- `pvm-storage`'s high-level model is **Solidity 32-byte slot addressing**: a
  value either fits in ≤8 static slots (≤256 B, striped → N SLOADs) or uses the
  dynamic-body path (header + `keccak(slot)+i` chunks → ~14 SLOADs per 400 B).
- OrderedIndex's perf invariant is **one node = one storage value = one read**.
  Both pvm-storage paths break that invariant for nodes >32 bytes.
- **Resolution:** add a minimal **raw-value primitive** to `pvm-storage` that
  wraps pallet-revive's variable-length `set_storage`/`get_storage` host API
  (≤416 B per key, one host op). Node serializes to a packed `Vec<u8>` ≤416 B
  and is stored via this primitive. This is generalizable (any opaque value
  benefits), not a hack.

## Design

### 1. Raw-value primitive (added to `crates/pvm-storage/src/lib.rs`)

```text
pub const MAX_STORAGE_VALUE_BYTES: usize = 416;   // pallet-revive STORAGE_BYTES cap

// alloc-gated; wrap host variable-length API
fn storage_get_bytes(host, key: &[u8;32]) -> Option<Vec<u8>>   // via HostApi::get_storage
fn storage_set_bytes(host, key: &[u8;32], value: &[u8])        // via HostApi::set_storage
fn storage_clear_value(host, key: &[u8;32])                    // set_storage_or_clear(zero)
```
- Missing key ↔ `None` (not zero) — distinct from the 32-byte "zero ≡ deleted"
  semantics. A stored empty `Vec` is represented by a 1-byte sentinel so it is
  distinguishable from absent (mirrors pvm-storage's inline-dynamic sentinel).
- One host SLOAD/SSTORE per node regardless of node byte length (≤416).

### 2. `OrderedIndex` module (`crates/pvm-storage/src/ordered_index.rs`)

```text
pub struct OrderedIndex<K, V, const T: usize = {computed default}>
where
    K: AsStorageKey + Ord + StorageEncode + StorageDecode,
    V: StorageEncode + StorageDecode,
```

- `K` and `V` are **SolType-native** (no SCALE). Public API mirrors Charles's:
  `new(namespace)`, `insert(k,v)->nonce`, `get_first(k)`, `remove_first(k)`,
  `remove(k,v)`, `remove_by_nonce(k,nonce)`, `select(rank)`, `rank_of_key(k)`,
  `range(from,to,offset,limit)`.
- Internal `Node<K,V>` + `Entry<K,V>` (no `#[derive(Encode,Decode)]`). Hand-rolled
  compact codec to/from `Vec<u8>` ≤416 B:
  - Header: `is_leaf:u8`, `entries_len:u16`, `children_len:u16`.
  - Each entry: `nonce:u64` + `K::encode` + `V::encode` (length-prefixed).
  - Each child: `NodeId:u64`, `subtree_count:u64`, `own_entry_count:u32`
    (the 20-byte-per-child mirror triple from Charles, packed tight).
  - `encoded_size()` + assert ≤ `MAX_STORAGE_VALUE_BYTES` on every write.
- Node storage via the raw-value primitive: key =
  `storage_derive_key(root, pad32(node_id))`; value = packed node bytes.
  `nodes()` / `load(id)` / `store(id,node)` / `free(id)` helpers.
- Root cell, next-id cell, next-nonce cell via `Lazy<u64>` (pvm-storage).
- B-tree algorithms (degree T): ported verbatim from Charles —
  `insert_nonfull`, `split_child`, `descend_prepared`, `borrow_from_left/right`,
  `merge_children`, `range` (in-order walk + early termination), `lower/upper_bound`.
  Complexity unchanged: O(log n) point ops, O(log n + items) range.

### 3. `T` (degree) default

Charles used `T=2` (3 entries/node) assuming 32-byte K+V. For username prefix
search the key is a lowercased display string (~15–30 B). The port computes a
const `T` from `K`/`V` storage size where possible, and exposes `T` as a const
generic so the optimization loop can sweep it. Initial default `T=2` to match
Charles (apples-to-apples baseline); the loop pushes T up.

## Test plan (Phase A3) — this is the optimization baseline

Property tests (via `proptest` or hand-rolled fast-check-style), all over a
seeded `MockHost`:
1. **Insert/get roundtrip:** every inserted `(k,v)` is reachable via `get_first`.
2. **Range completeness:** `range(k,k,0,∞)` returns exactly the inserted multiset
   for `k`, in `(k,nonce)` order.
3. **B-tree invariants:** every non-root node has `≥ T-1` and `≤ 2T-1` entries;
   leaves at equal depth; children counts consistent with `subtree_count` mirrors.
4. **416-byte cap:** no node ever encodes > `MAX_STORAGE_VALUE_BYTES` (assert in
   `store` + a property scanning all nodes).
5. **Remove idempotence:** `remove_first(k)` then `get_first(k)` is `None`.
6. **Cursor/range pagination:** `range` with `offset`/`limit` equals the slice of
   the full range.

## ce-optimize metric mapping (Phase B4 spec.yaml)

- `metric.primary`: `slot_reads_per_query` (hard, minimize) — counted via an
  instrumented `MockHost` wrapper around a prefix-range workload.
- `degenerate_gates`: correctness (range results == reference), no node > 416 B,
  all property tests pass.
- `diagnostics`: insert/remove slot-writes, range latency p50 (criterion), PG
  page-reads ratio, `.polkavm` binary-size delta.
- Postgres bar: real `postgres:18` in docker + the actual `searchUsernames`
  Drizzle query + `EXPLAIN(ANALYZE,BUFFERS)`; plus k6 e2e + EXPLAIN cost.

## Out of scope for this port (flagged)

- **#93 (derive-SolType-struct-as-Mapping-value):** OrderedIndex stores Node via
  the raw-value primitive, so it does NOT need `Mapping<K, derive(SolType) struct
  with Vec>`. #93 remains open for general user structs. Implementing #93's derive
  support is a separate workstream.
- `.polkavm` bytecode build (needs nightly + polkavm-linker): not required for
  MockHost-level benches; deferred to a final binary-size diagnostic if time.
