//! Host-side tests for the `OrderedIndex` B-tree.
//!
//! These run against the thread-local storage shim (see the `backend` module
//! in `storage.rs`), exercising the exact same tree code that runs on-chain.
//! Every structural property the on-chain code relies on is verified by
//! `check_invariants`:
//! - node fill bounds (root >= 1 entry, others >= T-1, all <= 2T-1),
//! - leaves all at the same depth,
//! - children / `child_counts` / `child_entry_counts` length consistency,
//! - `child_counts[i]` equals the recomputed subtree count of child `i`,
//! - `child_entry_counts[i]` equals child `i`'s own entry count,
//! - intra-node and global `(key, nonce)` sort order,
//! - every stored node encodes within `MAX_STORAGE_VALUE_BYTES`,
//! - `len()` agreement with the actual entry total.

use super::*;
use crate::storage::{MAX_STORAGE_VALUE_BYTES, host_storage_reset};
use alloc::format;
use alloc::string::String;
use core::fmt::Debug;
use core::ops::Bound;
use std::collections::BTreeMap;

// ------------------------------------------------------------------------
// Deterministic PRNG (xorshift64*), no external dependencies.
// ------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

// ------------------------------------------------------------------------
// Invariant verification
// ------------------------------------------------------------------------

struct SubtreeInfo {
    count: u64,
    entry_count: usize,
    depth: usize,
}

/// Recursively (host-side only; plenty of stack) verify the subtree rooted at
/// `id`, appending its entries in order to `in_order`.
fn verify_subtree<K, V, const T: usize>(
    idx: &OrderedIndex<K, V, T>,
    id: NodeId,
    is_root: bool,
    in_order: &mut Vec<(K, u64, V)>,
) -> SubtreeInfo
where
    K: Encode + Decode + Ord + Clone + Debug,
    V: Encode + Decode + Clone,
{
    let node = idx.load(id);

    assert!(
        node.entries.len() <= 2 * T - 1,
        "node {id} overfull: {} entries",
        node.entries.len()
    );
    if is_root {
        assert!(!node.entries.is_empty(), "stored root {id} is empty");
    } else {
        assert!(
            node.entries.len() >= T - 1,
            "non-root node {id} underfull: {} entries",
            node.entries.len()
        );
    }
    assert!(
        node.encoded_size() <= MAX_STORAGE_VALUE_BYTES,
        "node {id} exceeds the storage value cap"
    );
    for w in node.entries.windows(2) {
        assert!(
            w[0].key < w[1].key || (w[0].key == w[1].key && w[0].nonce < w[1].nonce),
            "node {id} entries out of (key, nonce) order"
        );
    }

    if node.is_leaf() {
        assert!(
            node.child_counts.is_empty() && node.child_entry_counts.is_empty(),
            "leaf {id} has mirror vectors"
        );
        for e in &node.entries {
            in_order.push((e.key.clone(), e.nonce, e.value.clone()));
        }
        return SubtreeInfo {
            count: node.entries.len() as u64,
            entry_count: node.entries.len(),
            depth: 1,
        };
    }

    let expected = node.entries.len() + 1;
    assert_eq!(node.children.len(), expected, "node {id} children length");
    assert_eq!(
        node.child_counts.len(),
        expected,
        "node {id} child_counts length"
    );
    assert_eq!(
        node.child_entry_counts.len(),
        expected,
        "node {id} child_entry_counts length"
    );

    let mut total = node.entries.len() as u64;
    let mut child_depth: Option<usize> = None;
    for i in 0..expected {
        let info = verify_subtree(idx, node.children[i], false, in_order);
        assert_eq!(
            node.child_counts[i], info.count,
            "node {id} child_counts[{i}] desynced from recomputed subtree count"
        );
        assert_eq!(
            node.child_entry_counts[i] as usize, info.entry_count,
            "node {id} child_entry_counts[{i}] desynced from child entry count"
        );
        match child_depth {
            None => child_depth = Some(info.depth),
            Some(d) => assert_eq!(d, info.depth, "node {id}: leaves at differing depths"),
        }
        total += info.count;
        if i < node.entries.len() {
            let e = &node.entries[i];
            in_order.push((e.key.clone(), e.nonce, e.value.clone()));
        }
    }

    SubtreeInfo {
        count: total,
        entry_count: node.entries.len(),
        depth: child_depth.unwrap() + 1,
    }
}

/// Full structural verification. Returns the global in-order entry list.
fn check_invariants<K, V, const T: usize>(idx: &OrderedIndex<K, V, T>) -> Vec<(K, u64, V)>
where
    K: Encode + Decode + Ord + Clone + Debug,
    V: Encode + Decode + Clone,
{
    let mut in_order = Vec::new();
    if let Some(root) = idx.root_id() {
        verify_subtree(idx, root, true, &mut in_order);
    }
    for w in in_order.windows(2) {
        assert!(
            w[0].0 < w[1].0 || (w[0].0 == w[1].0 && w[0].1 < w[1].1),
            "global (key, nonce) order violated"
        );
    }
    assert_eq!(
        idx.len(),
        in_order.len() as u64,
        "len() disagrees with walk"
    );
    in_order
}

fn tree_depth<K, V, const T: usize>(idx: &OrderedIndex<K, V, T>) -> usize
where
    K: Encode + Decode + Ord + Clone,
    V: Encode + Decode + Clone,
{
    let Some(mut id) = idx.root_id() else {
        return 0;
    };
    let mut depth = 1;
    loop {
        let node = idx.load(id);
        if node.is_leaf() {
            return depth;
        }
        id = node.children[0];
        depth += 1;
    }
}

/// `select(i)` returns the i-th smallest for sampled i, and rank/select are
/// inverses: `rank_of_key(select(i).key)` is the first in-order position of
/// that key.
fn check_rank_select<K, V, const T: usize>(
    idx: &OrderedIndex<K, V, T>,
    in_order: &[(K, u64, V)],
    rng: &mut Rng,
) where
    K: Encode + Decode + Ord + Clone + Debug,
    V: Encode + Decode + Clone + PartialEq + Debug,
{
    let n = in_order.len() as u64;
    assert!(idx.select(n).is_none(), "select(len) must be None");
    if n == 0 {
        return;
    }
    for _ in 0..24 {
        let r = rng.below(n);
        let (k, v) = idx.select(r).expect("select within bounds");
        assert_eq!(k, in_order[r as usize].0, "select({r}) key mismatch");
        assert_eq!(v, in_order[r as usize].2, "select({r}) value mismatch");
        let first = in_order.iter().position(|(kk, _, _)| *kk == k).unwrap() as u64;
        assert_eq!(idx.rank_of_key(&k), first, "rank/select inverse violated");
    }
}

/// Walk the whole index in fixed-size pages; the pages must concatenate to
/// exactly the in-order entry list.
fn check_range_pagination<K, V, const T: usize>(
    idx: &OrderedIndex<K, V, T>,
    in_order: &[(K, u64, V)],
    page: u64,
) where
    K: Encode + Decode + Ord + Clone + Debug,
    V: Encode + Decode + Clone + PartialEq + Debug,
{
    let mut collected: Vec<(K, V)> = Vec::new();
    let mut offset = 0u64;
    loop {
        let items = idx.range(Bound::Unbounded, Bound::Unbounded, offset, page);
        let got = items.len() as u64;
        collected.extend(items);
        if got < page {
            break;
        }
        offset += got;
    }
    assert_eq!(
        collected.len(),
        in_order.len(),
        "paged range total disagrees with entry count"
    );
    for (i, ((gk, gv), (wk, _, wv))) in collected.iter().zip(in_order.iter()).enumerate() {
        assert_eq!(gk, wk, "paged range key mismatch at {i}");
        assert_eq!(gv, wv, "paged range value mismatch at {i}");
    }
}

fn checkpoint<K, V, const T: usize>(idx: &OrderedIndex<K, V, T>, rng: &mut Rng)
where
    K: Encode + Decode + Ord + Clone + Debug,
    V: Encode + Decode + Clone + PartialEq + Debug,
{
    let in_order = check_invariants(idx);
    check_rank_select(idx, &in_order, rng);
    // Coarse pages at checkpoints keep each checkpoint linear; a
    // fine-grained pagination pass runs once at the end of each bulk test.
    let page = (in_order.len() as u64 / 4).max(1);
    check_range_pagination(idx, &in_order, page);
}

fn key_of(i: u32) -> String {
    format!("key{:05}", i)
}

// ------------------------------------------------------------------------
// Bulk insert tests: 10k unique String keys, T = 2
// ------------------------------------------------------------------------

const BULK: u32 = 10_000;
const CHECK_EVERY: u32 = 500;

fn run_bulk_string_inserts(namespace: &'static [u8], order: Vec<u32>) {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(namespace);
    let mut rng = Rng::new(0xDEADBEEF);
    for (n, i) in order.into_iter().enumerate() {
        idx.insert(&key_of(i), &i);
        if (n as u32 + 1) % CHECK_EVERY == 0 {
            checkpoint(&idx, &mut rng);
        }
    }
    assert_eq!(idx.len(), BULK as u64);
    let in_order = check_invariants(&idx);
    for (i, (k, _, v)) in in_order.iter().enumerate() {
        assert_eq!(*k, key_of(i as u32));
        assert_eq!(*v, i as u32);
    }
    // Fine-grained pagination over the full index once.
    check_range_pagination(&idx, &in_order, 999);
}

#[test]
fn ascending_inserts_10k() {
    run_bulk_string_inserts(b"asc10k", (0..BULK).collect());
}

#[test]
fn descending_inserts_10k() {
    run_bulk_string_inserts(b"desc10k", (0..BULK).rev().collect());
}

#[test]
fn seeded_random_inserts_10k() {
    let mut order: Vec<u32> = (0..BULK).collect();
    let mut rng = Rng::new(0x5EED);
    // Fisher-Yates shuffle.
    for i in (1..order.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        order.swap(i, j);
    }
    run_bulk_string_inserts(b"rand10k", order);
}

// ------------------------------------------------------------------------
// Duplicate keys: ordering by insertion nonce
// ------------------------------------------------------------------------

#[test]
fn duplicate_keys_order_by_nonce() {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"dups");
    let k = String::from("dup");
    let mut nonces = Vec::new();
    for i in 0..300u32 {
        nonces.push(idx.insert(&k, &i));
    }
    for w in nonces.windows(2) {
        assert!(w[0] < w[1], "nonces must be strictly increasing");
    }

    let in_order = check_invariants(&idx);
    assert_eq!(in_order.len(), 300);
    for (i, (kk, nonce, v)) in in_order.iter().enumerate() {
        assert_eq!(kk, &k);
        assert_eq!(*nonce, nonces[i], "entries must be ordered by nonce");
        assert_eq!(*v, i as u32, "values must come back in insertion order");
    }

    assert_eq!(idx.get_first(&k), Some(0));
    let items = idx.range(Bound::Included(&k), Bound::Included(&k), 0, u64::MAX);
    assert_eq!(items.len(), 300);
    for (i, (_, v)) in items.iter().enumerate() {
        assert_eq!(*v, i as u32);
    }

    // remove_first takes the earliest-inserted duplicate.
    assert_eq!(idx.remove_first(&k), Some(0));
    assert_eq!(idx.get_first(&k), Some(1));
    // remove_by_nonce hits an exact duplicate; a second call misses.
    assert_eq!(idx.remove_by_nonce(&k, nonces[150]), Some(150));
    assert_eq!(idx.remove_by_nonce(&k, nonces[150]), None);
    // remove(k, v) scans duplicates for the value.
    assert!(idx.remove(&k, &200));
    assert!(!idx.remove(&k, &200));
    check_invariants(&idx);
    assert_eq!(idx.len(), 297);
}

/// Playground-leaderboard tie scenario: thousands of distinct accounts at the
/// SAME score key. Verifies structural health under extreme duplication and
/// quantifies the read-cost asymmetry between `remove_by_nonce` (O(log n))
/// and value-based `remove(k, v)` (O(D * log n) — it must scan the duplicate
/// run for the value).
#[test]
fn extreme_duplication_leaderboard_tie() {
    use crate::storage::host_storage_read_count;

    host_storage_reset();
    let idx: OrderedIndex<u128, [u8; 20], 3> = OrderedIndex::new(b"ties");
    let tied_score = u128::MAX - 1_000;

    let addr = |i: u32| -> [u8; 20] {
        let mut a = [0u8; 20];
        a[..4].copy_from_slice(&i.to_be_bytes());
        a
    };

    // 5,000 accounts tied at one score, interleaved with 2,000 spread scores.
    let mut tie_nonces = Vec::new();
    for i in 0..5_000u32 {
        tie_nonces.push(idx.insert(&tied_score, &addr(i)));
        if i % 5 == 0 {
            idx.insert(&(u128::MAX - 2_000 - i as u128), &addr(i));
        }
    }
    assert_eq!(idx.len(), 6_000);
    let in_order = check_invariants(&idx);

    // All 5,000 duplicates are adjacent and ordered by insertion nonce.
    let dup_run: Vec<_> = in_order
        .iter()
        .filter(|(k, _, _)| *k == tied_score)
        .collect();
    assert_eq!(dup_run.len(), 5_000);
    for w in dup_run.windows(2) {
        assert!(w[0].1 < w[1].1);
    }
    // rank_of_key is exact even inside a huge duplicate run.
    assert_eq!(
        idx.rank_of_key(&tied_score),
        in_order
            .iter()
            .position(|(k, _, _)| *k == tied_score)
            .unwrap() as u64
    );

    // Nonce-based removal: O(log n) reads, independent of duplicate count.
    let before = host_storage_read_count();
    assert_eq!(
        idx.remove_by_nonce(&tied_score, tie_nonces[2_500]),
        Some(addr(2_500))
    );
    let nonce_reads = host_storage_read_count() - before;

    // Value-based removal of an account deep in the duplicate run: must scan.
    let before = host_storage_read_count();
    assert!(idx.remove(&tied_score, &addr(4_999)));
    let value_reads = host_storage_read_count() - before;

    std::eprintln!(
        "extreme-dup reads: remove_by_nonce={nonce_reads}, remove(k,v) deep in 5k run={value_reads}"
    );
    assert!(
        nonce_reads < 64,
        "remove_by_nonce should be O(log n) reads, got {nonce_reads}"
    );
    assert!(
        value_reads > nonce_reads * 10,
        "expected value-based removal to scan the duplicate run \
         (nonce: {nonce_reads} reads, value: {value_reads} reads)"
    );

    check_invariants(&idx);
    assert_eq!(idx.len(), 5_998);
}

// ------------------------------------------------------------------------
// Mixed insert/remove workload against a reference model
// ------------------------------------------------------------------------

fn compare_with_model(in_order: &[(String, u64, u32)], model: &BTreeMap<(String, u64), u32>) {
    assert_eq!(
        in_order.len(),
        model.len(),
        "entry count disagrees with model"
    );
    for (got, (mk, mv)) in in_order.iter().zip(model.iter()) {
        assert_eq!(got.0, mk.0, "key disagrees with model");
        assert_eq!(got.1, mk.1, "nonce disagrees with model");
        assert_eq!(got.2, *mv, "value disagrees with model");
    }
}

#[test]
fn mixed_ops_20k_with_model() {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"mixed20k");
    let mut model: BTreeMap<(String, u64), u32> = BTreeMap::new();
    let mut rng = Rng::new(0xC0FFEE);

    for op in 0..20_000u32 {
        let roll = rng.below(100);
        if roll < 55 || model.is_empty() {
            // Insert; the small key space forces heavy duplication.
            let k = format!("k{:03}", rng.below(400));
            let v = rng.next() as u32;
            let nonce = idx.insert(&k, &v);
            assert!(
                model.insert((k, nonce), v).is_none(),
                "insert returned a reused nonce"
            );
        } else if roll < 80 {
            // remove_by_nonce of a random live entry.
            let pick = rng.below(model.len() as u64) as usize;
            let (k, nonce) = model.keys().nth(pick).cloned().unwrap();
            let want = model.remove(&(k.clone(), nonce)).unwrap();
            assert_eq!(idx.remove_by_nonce(&k, nonce), Some(want));
        } else if roll < 90 {
            // remove(k, v): must take the lowest-nonce entry matching both.
            let pick = rng.below(model.len() as u64) as usize;
            let ((k, _), v) = model
                .iter()
                .nth(pick)
                .map(|(k, v)| (k.clone(), *v))
                .unwrap();
            let first = model
                .range((k.clone(), 0)..=(k.clone(), u64::MAX))
                .find(|(_, mv)| **mv == v)
                .map(|((mk, mn), _)| (mk.clone(), *mn))
                .unwrap();
            assert!(idx.remove(&k, &v));
            model.remove(&first).unwrap();
        } else {
            // Aborting path: removing a missing (key, nonce) must be a no-op
            // (the count mirrors are checked at the next checkpoint).
            let k = format!("k{:03}", rng.below(400));
            assert_eq!(idx.remove_by_nonce(&k, u64::MAX - 1), None);
            assert_eq!(idx.remove_by_nonce(&String::from("zzz-missing"), 7), None);
        }

        if (op + 1) % 1000 == 0 {
            let in_order = check_invariants(&idx);
            compare_with_model(&in_order, &model);
        }
    }

    // Drain to empty via remove_by_nonce, verifying along the way.
    let mut drained = 0u32;
    while let Some(((k, nonce), v)) = model.pop_first() {
        assert_eq!(idx.remove_by_nonce(&k, nonce), Some(v));
        drained += 1;
        if drained % 1000 == 0 {
            let in_order = check_invariants(&idx);
            compare_with_model(&in_order, &model);
        }
    }
    assert!(idx.is_empty());
    assert_eq!(idx.len(), 0);
    assert!(idx.root_id().is_none(), "root cell must be cleared");
    assert!(idx.select(0).is_none());
    check_invariants(&idx);

    // The tree must remain usable after being emptied.
    idx.insert(&String::from("rebirth"), &1);
    assert_eq!(idx.get_first(&String::from("rebirth")), Some(1));
    check_invariants(&idx);
}

// ------------------------------------------------------------------------
// Regression: T = 2 height transition around insert #36
// ------------------------------------------------------------------------

/// On-chain, the old recursive implementation trapped (stack overflow under
/// the 8 KiB PolkaVM default stack) at insert #36 for
/// `OrderedIndex<String, u32, 2>` fed sequential keys (the registry-index
/// stress workload). This walks 50 sequential inserts - spanning every height
/// transition up to depth 5, and in particular both sides of insert #36 -
/// with full structural invariants verified after every single insert.
///
/// The exact transition counts are asserted too: depth 2/3/4/5 is first
/// reached at insert 4/9/18/35, so the tree grows to height 5 exactly at the
/// count-35/36 boundary where the chain trapped. Pinning these counts also
/// guards storage layout compatibility: the preemptive-split descent must
/// keep producing the same shapes as the previous recursive implementation
/// for already-deployed trees.
#[test]
fn height_transition_regression_t2() {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"regress50");
    let mut depth_at: Vec<usize> = Vec::new();
    for i in 0..50u32 {
        idx.insert(&format!("contract-name-{:02}", i), &i);
        let in_order = check_invariants(&idx);
        assert_eq!(in_order.len() as u64, u64::from(i) + 1);
        depth_at.push(tree_depth(&idx));
    }
    for w in depth_at.windows(2) {
        assert!(w[1] >= w[0], "tree depth must never shrink during inserts");
    }
    let first_at = |depth: usize| depth_at.iter().position(|d| *d == depth).map(|p| p + 1);
    assert_eq!(first_at(2), Some(4), "height-1 -> 2 transition moved");
    assert_eq!(first_at(3), Some(9), "height-2 -> 3 transition moved");
    assert_eq!(first_at(4), Some(18), "height-3 -> 4 transition moved");
    assert_eq!(
        first_at(5),
        Some(35),
        "height-4 -> 5 transition moved off the count-35/36 boundary"
    );
    assert_eq!(*depth_at.last().unwrap(), 5);
}

// ------------------------------------------------------------------------
// Playground-shaped index: u128 keys, [u8; 20] values, T = 3
// ------------------------------------------------------------------------

#[test]
fn u128_address_t3_5k() {
    host_storage_reset();
    let idx: OrderedIndex<u128, [u8; 20], 3> = OrderedIndex::new(b"play5k");
    let mut rng = Rng::new(0xAB1E);
    let mut model: BTreeMap<(u128, u64), [u8; 20]> = BTreeMap::new();

    for i in 0..5_000u32 {
        let k = ((rng.next() as u128) << 64) | rng.next() as u128;
        let mut v = [0u8; 20];
        v[..8].copy_from_slice(&rng.next().to_le_bytes());
        v[8..16].copy_from_slice(&rng.next().to_le_bytes());
        let nonce = idx.insert(&k, &v);
        model.insert((k, nonce), v);
        if (i + 1) % 500 == 0 {
            let in_order = check_invariants(&idx);
            check_rank_select(&idx, &in_order, &mut rng);
            for (got, ((mk, mn), mv)) in in_order.iter().zip(model.iter()) {
                assert_eq!(got.0, *mk);
                assert_eq!(got.1, *mn);
                assert_eq!(got.2, *mv);
            }
        }
    }
    assert_eq!(idx.len(), 5_000);
    let in_order = check_invariants(&idx);
    check_range_pagination(&idx, &in_order, 777);
}

// ------------------------------------------------------------------------
// Encoded node size guard
// ------------------------------------------------------------------------

#[test]
fn full_node_with_64_byte_string_keys_fits_t2() {
    // Worst-case T = 2 internal node: 2T-1 = 3 entries, each with a 64-byte
    // string key, max-varint nonce and u32 value, plus 2T = 4 child links and
    // both mirror vectors. This is the shape `store` must accept for the CDM
    // registry's package-name index.
    let key: String = core::iter::repeat('x').take(64).collect();
    let node: Node<String, u32> = Node {
        entries: (0..3u64)
            .map(|i| Entry {
                key: key.clone(),
                nonce: u64::MAX - i,
                value: u32::MAX,
            })
            .collect(),
        children: alloc::vec![NodeId::MAX; 4],
        child_counts: alloc::vec![u64::MAX; 4],
        child_entry_counts: alloc::vec![u32::MAX; 4],
    };
    assert!(
        node.encoded_size() <= MAX_STORAGE_VALUE_BYTES,
        "full T=2 node with 64-byte String keys must fit one storage value \
         ({} > {MAX_STORAGE_VALUE_BYTES})",
        node.encoded_size()
    );

    // The const envelope formula must match real encoding exactly: a 64-byte
    // String key encodes as 2 (compact prefix) + 64 bytes.
    assert_eq!(
        OrderedIndex::<String, u32, 2>::max_node_encoded_size(66, 4),
        node.encoded_size(),
    );
    assert!(OrderedIndex::<String, u32, 2>::fits_storage_limit(66, 4));
    // Playground shape: u128 key + 20-byte address. T = 4 cannot fit a full
    // node within the 416-byte storage limit; T = 3 can.
    assert!(!OrderedIndex::<u128, [u8; 20], 4>::fits_storage_limit(
        16, 20
    ));
    assert!(OrderedIndex::<u128, [u8; 20], 3>::fits_storage_limit(
        16, 20
    ));
}

// ------------------------------------------------------------------------
// Edge cases and aborting paths
// ------------------------------------------------------------------------

#[test]
fn empty_and_missing_key_behaviour() {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"edge");
    let k = String::from("nope");

    assert!(idx.is_empty());
    assert_eq!(idx.len(), 0);
    assert_eq!(idx.get_first(&k), None);
    assert!(!idx.contains_key(&k));
    assert_eq!(idx.remove_by_nonce(&k, 0), None);
    assert_eq!(idx.remove_first(&k), None);
    assert!(!idx.remove(&k, &1));
    assert!(idx.select(0).is_none());
    assert_eq!(idx.rank_of_key(&k), 0);
    assert!(
        idx.range(Bound::Unbounded, Bound::Unbounded, 0, 10)
            .is_empty()
    );

    // A failed (aborting) remove must leave the tree byte-identical: the
    // destructive pass only runs after the existence pre-check passes.
    for i in 0..100u32 {
        idx.insert(&format!("k{:02}", i % 30), &i);
    }
    let before = check_invariants(&idx);
    assert_eq!(idx.remove_by_nonce(&String::from("k05"), 9_999_999), None);
    assert_eq!(idx.remove_by_nonce(&String::from("zzz"), 0), None);
    let after = check_invariants(&idx);
    assert_eq!(before, after, "aborted remove modified the tree");

    // Single-entry tree removal clears the root cell.
    let single: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"edge-single");
    let nonce = single.insert(&k, &7);
    assert_eq!(single.remove_by_nonce(&k, nonce), Some(7));
    assert!(single.is_empty());
    assert!(single.root_id().is_none());
}

#[test]
fn range_bounds_and_offsets() {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"rangebounds");
    for i in 0..100u32 {
        idx.insert(&format!("k{:03}", i), &i);
    }
    let k = |i: u32| format!("k{:03}", i);

    let values = |items: Vec<(String, u32)>| items.into_iter().map(|(_, v)| v).collect::<Vec<_>>();

    // [k010, k020)
    let got = values(idx.range(
        Bound::Included(&k(10)),
        Bound::Excluded(&k(20)),
        0,
        u64::MAX,
    ));
    assert_eq!(got, (10..20).collect::<Vec<_>>());
    // (k010, k020]
    let got = values(idx.range(
        Bound::Excluded(&k(10)),
        Bound::Included(&k(20)),
        0,
        u64::MAX,
    ));
    assert_eq!(got, (11..21).collect::<Vec<_>>());
    // Offset + limit pagination inside a bounded range.
    let got = values(idx.range(Bound::Included(&k(10)), Bound::Included(&k(30)), 5, 3));
    assert_eq!(got, alloc::vec![15, 16, 17]);
    // Zero limit short-circuits.
    assert!(
        idx.range(Bound::Unbounded, Bound::Unbounded, 0, 0)
            .is_empty()
    );
    // Offset past the end yields nothing.
    assert!(
        idx.range(Bound::Unbounded, Bound::Unbounded, 1000, 10)
            .is_empty()
    );
    // Bounds beyond either edge.
    let got = values(idx.range(
        Bound::Included(&String::from("a")),
        Bound::Excluded(&k(2)),
        0,
        u64::MAX,
    ));
    assert_eq!(got, alloc::vec![0, 1]);
    let got = values(idx.range(Bound::Included(&k(98)), Bound::Unbounded, 0, u64::MAX));
    assert_eq!(got, alloc::vec![98, 99]);
}

// ------------------------------------------------------------------------
// Positional offset skipping in range()
// ------------------------------------------------------------------------

/// Reference `range` semantics computed from the global in-order entry list:
/// filter by both bounds, skip `offset` *in-range* entries, take `limit`.
fn reference_range<K, V>(
    in_order: &[(K, u64, V)],
    from: Bound<&K>,
    to: Bound<&K>,
    offset: u64,
    limit: u64,
) -> Vec<(K, V)>
where
    K: Ord + Clone,
    V: Clone,
{
    in_order
        .iter()
        .filter(|(k, _, _)| {
            (match from {
                Bound::Unbounded => true,
                Bound::Included(b) => k >= b,
                Bound::Excluded(b) => k > b,
            }) && (match to {
                Bound::Unbounded => true,
                Bound::Included(b) => k <= b,
                Bound::Excluded(b) => k < b,
            })
        })
        .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
        .map(|(k, _, v)| (k.clone(), v.clone()))
        .collect()
}

/// Every (from, to, offset, limit) combination over a 10k-entry tree must
/// match the reference exactly - in particular deep offsets (the positional
/// seek path), offsets landing exactly on / past the range end, and Excluded
/// bounds. Grid: 8 offsets x 4 limits x 3 from-kinds x 3 to-kinds, over both
/// a mid/mid window and a lo/hi window.
#[test]
fn deep_offset_range_equivalence_10k() {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"deepoff10k");
    for i in 0..BULK {
        idx.insert(&key_of(i), &i);
    }
    let in_order = check_invariants(&idx);

    let mid = key_of(BULK / 2);
    let lo = key_of(3_000);
    let hi = key_of(7_000);
    let offsets = [0u64, 1, 7, 500, 5_000, 9_999, 10_000, 10_007];
    let limits = [0u64, 1, 13, 100];

    for (from_key, to_key) in [(&mid, &mid), (&lo, &hi)] {
        let from_kinds = [
            Bound::Unbounded,
            Bound::Included(from_key),
            Bound::Excluded(from_key),
        ];
        let to_kinds = [
            Bound::Unbounded,
            Bound::Included(to_key),
            Bound::Excluded(to_key),
        ];
        for from in from_kinds {
            for to in to_kinds {
                for offset in offsets {
                    for limit in limits {
                        let got = idx.range(from, to, offset, limit);
                        let want = reference_range(&in_order, from, to, offset, limit);
                        assert_eq!(
                            got, want,
                            "range({from:?}, {to:?}, offset={offset}, limit={limit}) \
                             diverged from reference"
                        );
                    }
                }
            }
        }
    }
}

/// 5,000 entries on ONE key plus 1,000 spread keys (half sorting below the
/// run, half above). Used by the duplicate-run pagination and read-cost
/// tests below. Returns the duplicated key.
fn build_duplicate_run_tree(idx: &OrderedIndex<String, u32, 2>) -> String {
    let dup = String::from("m-dup");
    for i in 0..5_000u32 {
        idx.insert(&dup, &i);
        if i % 5 == 0 {
            // Alternate spread keys below ("a...") and above ("z...") the run.
            let spread = if i % 10 == 0 {
                format!("a{:05}", i)
            } else {
                format!("z{:05}", i)
            };
            idx.insert(&spread, &i);
        }
    }
    assert_eq!(idx.len(), 6_000);
    dup
}

/// Deep-offset pagination *inside* a 5k-duplicate run: the positional seek
/// must land on the right nonce-ordered duplicate, not just the right key.
#[test]
fn duplicate_run_deep_offset_pagination() {
    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"dupdeepoff");
    let dup = build_duplicate_run_tree(&idx);
    let in_order = check_invariants(&idx);

    for offset in [0u64, 2_500, 4_999] {
        for limit in [1u64, 13, 100] {
            // Page within the run via key-bounded ranges.
            let from = Bound::Included(&dup);
            let to = Bound::Included(&dup);
            let got = idx.range(from, to, offset, limit);
            let want = reference_range(&in_order, from, to, offset, limit);
            assert_eq!(got, want, "dup-run page offset={offset} limit={limit}");
            // Duplicates come back in insertion (nonce) order: the values
            // were inserted as 0..5000, so page `offset` starts at value
            // `offset`.
            assert_eq!(got[0].1, offset as u32);
            assert_eq!(got.len() as u64, limit.min(5_000 - offset));

            // And via unbounded ranges cutting into the run (the 500 "a"
            // spread keys sort before it).
            let got = idx.range(Bound::Unbounded, Bound::Unbounded, 500 + offset, limit);
            let want = reference_range(
                &in_order,
                Bound::Unbounded,
                Bound::Unbounded,
                500 + offset,
                limit,
            );
            assert_eq!(got, want, "unbounded page offset={offset} limit={limit}");
            assert_eq!(got[0].0, dup);
        }
    }

    // Offset exactly at / past the end of the duplicate run.
    assert!(
        idx.range(Bound::Included(&dup), Bound::Included(&dup), 5_000, 10)
            .is_empty()
    );
    assert!(
        idx.range(Bound::Included(&dup), Bound::Excluded(&dup), 0, 10)
            .is_empty()
    );
}

/// Offset skipping must be positional - O(log n) reads - not a walk paying
/// one storage read per skipped entry. On a 6,000-entry tree a deep-offset
/// page must stay within a small absolute read budget and within 2x of a
/// zero-offset page plus O(log n) slack.
#[test]
fn deep_offset_read_cost() {
    use crate::storage::host_storage_read_count;

    host_storage_reset();
    let idx: OrderedIndex<String, u32, 2> = OrderedIndex::new(b"dupdeepreads");
    let dup = build_duplicate_run_tree(&idx);
    let in_order = check_invariants(&idx);

    let before = host_storage_read_count();
    let shallow = idx.range(Bound::Unbounded, Bound::Unbounded, 0, 10);
    let shallow_reads = host_storage_read_count() - before;
    assert_eq!(
        shallow,
        reference_range(&in_order, Bound::Unbounded, Bound::Unbounded, 0, 10)
    );

    let before = host_storage_read_count();
    let deep = idx.range(Bound::Unbounded, Bound::Unbounded, 5_000, 10);
    let deep_reads = host_storage_read_count() - before;
    assert_eq!(
        deep,
        reference_range(&in_order, Bound::Unbounded, Bound::Unbounded, 5_000, 10)
    );
    assert_eq!(deep.len(), 10);
    assert_eq!(deep[0].0, dup, "rank 5000 sits inside the duplicate run");

    std::eprintln!(
        "range reads on 6k entries (limit 10): offset=0 -> {shallow_reads}, \
         offset=5000 -> {deep_reads}"
    );
    assert!(
        deep_reads < 64,
        "deep-offset range must cost O(log n + limit) reads, got {deep_reads}"
    );
    // The deep call pays at most one extra positional descent over the
    // shallow call: within 2x of the shallow cost plus tree-depth slack.
    assert!(
        deep_reads <= 2 * shallow_reads + 32,
        "deep-offset reads ({deep_reads}) not within 2x of zero-offset reads \
         ({shallow_reads}) + O(log n)"
    );
}
