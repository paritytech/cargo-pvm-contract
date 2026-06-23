---
title: Adaptive byte-budget fanout for variable-size B-tree nodes under a storage byte cap
date: 2026-06-23
category: docs/solutions/design-patterns
module: pvm-storage / OrderedIndex
problem_type: design_pattern
component: tooling
severity: medium
applies_when:
  - A B-tree serializes each node into a fixed byte budget (pallet-revive storage has a 416-byte per-value cap)
  - Entries are variable-size (string key suffixes, compressed values, varint-encoded counts)
  - Range-query cost is dominated by the number of storage reads, which is set by tree height and node fanout
tags: [b-tree, ordered-index, pvm-storage, pallet-revive, polkavm, storage-weight, fanout, byte-budget]
---

# Adaptive byte-budget fanout for variable-size B-tree nodes under a storage byte cap

## Context

`pvm-storage`'s `OrderedIndex` is a B-tree-backed ordered range index for PolkaVM contracts on pallet-revive. pallet-revive charges weight per storage read/write/clear (sourced from the asset-hub-westend `DbWeight`) and enforces a hard **416-byte per-storage-value cap** — any single node that encodes larger traps the contract.

The original implementation used the textbook CLRS fixed-degree scheme: a node splits when its **entry count** reaches `2T-1`. That scheme is correct and optimal **for fixed-size entries**. `OrderedIndex` entries are variable-size (varint nonce, length-prefixed key suffix, length-prefixed value), so a count-based split leaves nodes roughly half-full by *byte* budget. At `T=10`, a full node held ~19 entries averaging ~20 B — about 380 B out of the 416 B available, but only because the count cap fired before the byte budget did. Leaf nodes with good prefix sharing could have held 30–40+ entries.

The observable cost: at N=1,000,000 records and `T=10`, a prefix range query cost **995,118,621 ps** of storage weight and **14.31 storage reads** per query. The count-based fanout was the dominant lever — node fill ratio, not codec density or tree degree.

## Guidance

For variable-size B-tree nodes under a byte budget, replace the top-down count-based proactive split with a **bottom-up byte-budget split**. The byte budget — not the entry count — decides when a node is full.

### The pattern

1. `insert` recurses toward the leaf and inserts the entry.
2. After the insert (and after absorbing any `ChildSplit` propagated up from a child), check `node.encode().len() > MAX_STORAGE_VALUE_BYTES`.
3. On overflow: split the node at the **byte-balanced** cut point — scan from the middle outward until both halves encode to ≤ the cap. Store the left half in place, allocate a new node for the right half, and return a `ChildSplit { median_entry, right_id, left/right subtree counts }` to the caller.
4. The parent inserts the median entry and the `right_id` into its own entry/child arrays. If the parent now overflows, it repeats step 3 — the split propagates upward at most to the root, where a new root is allocated.
5. `T` is demoted to a **balance floor** only (minimum entries per node = `T-1`); it no longer determines fanout. The byte budget does. Measured weight becomes T-independent — `T=2`, `T=4`, and `T=10` all produce identical query weight.

### Why bottom-up, not top-down

CLRS proactive splitting (split any full child on the way *down*) is only safe for fixed-size entries. With variable-size entries, the median promoted out of a child split can unpredictably overflow the parent, and a top-down pass has already committed to a path. **Bottom-up** (insert → detect overflow → propagate the split upward) is inherently safe because each split decision is made with the actual encoded size in hand. Every production variable-size B-tree — SQLite's b-tree pages, PostgreSQL's GiST/B-tree pages, Btrfs nodes — splits bottom-up for this reason.

### Two correctness traps that bit during implementation

- **The encode-size assert must live in the store path, not the encode path.** If `encode()` itself asserts `total <= cap`, it panics *before* the overflow check in `insert` can fire — the splitter never runs. Keep one size assert in the store routine (the sole enforcer) and let `encode()` return the true length.
- **Refresh child mirrors after a non-splitting child insert.** When a recursive child insert returns `None` (no split) but the child still *grew* (gained an entry), the parent's cached child subtree-count and own-entry-count are now stale. Without refreshing them, `len()` undercounts and later splits compute wrong balance points — a silent data-loss bug. After handling a `None` return from a child, refresh that child's mirror fields from the child's current state.

The per-node entry-count cap becomes a `u8` (255) sanity ceiling, replacing the old `2T-1` ceiling. The property test for the B-tree invariant changes from `entries.len() <= 2T-1` to `encode().len() <= MAX_STORAGE_VALUE_BYTES` — assert the actual constraint the system enforces.

## Why This Matters

Switching from count-based to byte-budget fanout, at N=1,000,000 records and Q=1,000 range queries:

| Metric | Before (count-based) | After (byte-budget) | Δ |
| --- | --- | --- | --- |
| weight_ref_time_per_query | 995,118,621 ps | 703,968,859 ps | **−29.2%** |
| weight_proof_size_per_query | 148,575 B | 105,936 B | −28.7% |
| slot_reads_per_query | 14.31 | 10.10 | −29.4% |
| insert_weight_ref_time | 2.57e15 ps | 1.84e15 ps | −28.3% |

Node fill ratio rose from ~50% to ~92%. The 10.10 reads/query decompose as roughly 4 descent + 4 leaf + 2 navigation — within reach of the B-tree floor. The optimization target (850M ps) was exceeded by 17%.

The dominant lever was **node fill ratio**. Codec density (already varint + prefix-compressed + a one-byte own-entry-count) and tree degree were second-order; the codec was near-minimal before this change, and `T` became irrelevant after it. The lesson generalizes: for byte-budgeted storage, measure fill ratio first — it usually dwarfs encoding micro-optimizations.

## When to Apply

- **Any B-tree whose nodes serialize into a fixed byte budget** — pallet-revive contract storage (416 B cap), database page sizes, filesystem block sizes, on-chain trie nodes.
- **Entries are variable-size** (strings, compressed payloads, varint counts). This is the precondition that makes count-based splits leave nodes half-full.
- **Range-scan cost is read-dominated** (each node touched is one storage read / page fault).

Do **not** apply when entries are fixed-size — there, count-based splits already fill nodes to the budget, and the added split-scanning logic is pure overhead. A CLRS count-based tree remains the right default for fixed-size keys/values.

## Examples

### Before — count-based proactive split (conceptual)

```text
insert(node, entry):
  # split full children on the way DOWN (CLRS)
  if child is full (entries.len() == 2*T - 1):
    split_child(node, child_idx)     # split by COUNT, not bytes
  descend, insert_nonfull(child, entry)
# node may be encoded far below the 416 B cap — wasted fanout
```

### After — byte-budget bottom-up split (conceptual)

```text
insert_rec(node_id, entry) -> Option<ChildSplit>:
  node = load(node_id)
  if node.is_leaf():
    node.insert_entry(entry)
  else:
    child_idx = pick_child(node, entry.key)
    match insert_rec(node.child(child_idx), entry):
      Some(split) => node.absorb_child_split(child_idx, split)  # insert median + right_id
      None        => node.refresh_child_mirrors(child_idx)      # child grew without splitting
  # the byte budget — not the count — decides fullness
  if node.encode().len() > MAX_STORAGE_VALUE_BYTES:
    Some(node.split_bytes_balance())   # both halves ≤ cap; return median + right_id upward
  else:
    store(node_id, node)
    None
```

`split_bytes_balance` scans from the middle cut outward, accepting the first cut where both the left and right encoded halves fit under the cap (lower bound `T-1` entries per half, upper bound leaves room for the median).

### Measurement harness

Query weight is measured with the `measure-ordered-index` binary, which drives a `CountingHost` that records per-op byte sizes and accumulates the real pallet-revive `Weight` (ref_time + proof_size) using asset-hub-westend `DbWeight` constants. The metric is **storage-ops-only** — a deliberate lower bound. An on-chain validation against `revive-dev-node` (a real pallet-revive node) measured the full gas at ~96× the storage-ops model: the gap is frame/dispatch setup, ABI decode, PolkaVM compute, and the full host-call path per access. The storage-ops metric is therefore valid as a **relative** optimization proxy (the fixed compute overhead is constant across T/codec variants) but is **not** an absolute gas predictor.

## Related

- Supporting commit on this branch: routing `OrderedIndex` admin cells through the Solidity `mapping(string => bytes)` key preimage, so every `pvm-storage` key derivation is Solidity-compatible (previously the admin cells used a bespoke keccak preimage that `cast`/foundry could not compute). Enforced by a cast-parity test mirroring the crate's existing Solidity-layout tests for `Lazy`/`Mapping`/`StorageVec`.
- Supporting commit on this branch: an end-to-end on-chain gas validation harness (`examples/ordered-index-bench` contract + a `pvm-contract-e2e-tests` case against `revive-dev-node`) that produced the ~96× gap figure above and pins the metric's "storage-ops lower bound" framing.
- The weight model itself (per-op read/write/clear ref_time + proof_size constants) is captured in the `pallet_revive_weight` bench module, with every constant cited to its polkadot-sdk source.
