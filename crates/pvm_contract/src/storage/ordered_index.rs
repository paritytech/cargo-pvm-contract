//! Persistent sorted multimap with O(log n) ops over `pvm_contract`'s key-value storage.
//!
//! A B-tree of minimum degree `T` (default 2): each node holds between `T-1` and
//! `2T-1` entries and occupies exactly one storage slot. Because `pallet-revive`
//! caps storage values at 416 bytes and charges a ~9.5M-ps base cost per slot
//! access, packing many entries per slot - rather than one node per entry - is
//! the right on-chain shape for a search tree. Read/write cost scales with the
//! height of the tree, which is ~log_T(n): even `T = 2` keeps a million entries
//! to only on the order of tens of levels, and larger `T` shrinks that further.
//! Every node write checks the encoded node size before touching storage, so
//! oversized variable-length keys fail with a clean revert instead of relying on
//! the host to reject the write.
//!
//! Duplicate keys are allowed. Internally every entry gets a monotonic insertion
//! nonce, so `(K, nonce)` forms a strict total order; the public API remains
//! keyed on `K` alone.
//!
//! ## Picking `T`
//! Each entry costs ~`size_of(K) + size_of(V) + 8` bytes; each child link costs
//! `20` bytes (8-byte id + 8-byte mirrored subtree count + 4-byte mirrored
//! own-entry count). Keep `2T-1` entries plus `2T` child links under ~400 bytes:
//! - Fixed 32-byte K and 32-byte V: `T = 2` (3 entries, 4 children) is the safe
//!   default. T=3 risks exceeding the 416-byte cap on internal nodes.
//! - Variable-length K (package names, etc.): `T = 2` is again the safe choice.
//! - Small fixed K like `u32`/`u64` with a 32-byte V: `T = 3` fits comfortably.
//!
//! ## Complexity
//! - `insert`, `remove_by_nonce`, `get_first`, `select`, `rank_of_key`: O(log n).
//! - `remove_first(k)`: O(log n).
//! - `remove(k, v)`: O(D * log n) - one descent per duplicate inspected. For
//!   hot paths with heavy duplication, keep the nonce returned by `insert`
//!   and call `remove_by_nonce` instead.
//! - `range(...)`: O(log n + limit). The pagination `offset` is consumed
//!   positionally via the mirrored subtree counts (a select-style descent),
//!   so deep offsets cost tree-depth reads, not one read per skipped entry.
//!
//! ## Stack safety
//! PolkaVM guests get a small fixed stack (8 KiB by default), so **nothing in
//! this module recurses**. Every operation is an iterative loop holding O(1)
//! nodes in memory:
//! - `insert` is the classic single-pass preemptive-split descent: any full
//!   child encountered on the way down is split while its parent is
//!   guaranteed non-full, so no back-propagation is ever needed.
//! - `remove_by_nonce` is the CLRS single-pass descent that rebalances
//!   (borrow/merge) on the way down. When the doomed entry is found in an
//!   internal node it is replaced by its in-order predecessor/successor; the
//!   descent simply switches to "extract the max/min of this subtree" mode
//!   and patches the replacement into the recorded slot at the very end.
//! - `range` drives an explicit `Vec<(node id, position)>` cursor stack (two
//!   machine words per tree level) instead of call recursion; only one node
//!   is decoded at a time. The stack is seeded by an iterative positional
//!   descent to the offset's global rank, never by walking entries.
//!
//! The mirrored `child_counts` / `child_entry_counts` are updated strictly on
//! the way down: a successful insert adds exactly one entry to the descended
//! subtree and a remove takes exactly one away, so each parent mirror can be
//! adjusted as the descent passes through. Because a remove of a missing
//! `(key, nonce)` would otherwise leave those pre-decremented counts corrupt,
//! `remove_by_nonce` first runs a cheap read-only descent to confirm the
//! entry exists and only then performs the destructive (guaranteed to
//! succeed) pass — aborting paths therefore never touch the counts.

use alloc::vec::Vec;
use core::marker::PhantomData;
use core::ops::Bound;
use parity_scale_codec::{Decode, Encode};

use super::{Lazy, MAX_STORAGE_VALUE_BYTES, Mapping, namespaced_key, revert};

/// Node identifier. Allocated monotonically from 1 upward; 0 means "null".
type NodeId = u64;

/// One logical entry in the index. The `nonce` breaks ties among equal `key`s
/// so `(key, nonce)` forms a strict total order across the whole tree.
#[derive(Encode, Decode, Clone)]
struct Entry<K, V> {
    key: K,
    nonce: u64,
    value: V,
}

/// A B-tree node, stored as a single storage value.
///
/// Invariants:
/// - `entries` is sorted by `(key, nonce)`.
/// - For a leaf: `children`, `child_counts`, and `child_entry_counts` are all empty.
/// - For an internal node: all three vecs share the same length, equal to
///   `entries.len() + 1`.
/// - `child_counts[i]` mirrors `children[i].subtree_count()` (used by
///   rank/select and `range`'s positional offset seek).
/// - `child_entry_counts[i]` mirrors `children[i].entries.len()` (used by the
///   delete rebalancer, which needs each child's own key count - *not* its
///   subtree size - to enforce the B-tree minimum-key invariant).
#[derive(Encode, Decode, Clone)]
struct Node<K, V> {
    entries: Vec<Entry<K, V>>,
    children: Vec<NodeId>,
    child_counts: Vec<u64>,
    child_entry_counts: Vec<u32>,
}

impl<K, V> Node<K, V> {
    fn leaf() -> Self {
        Self {
            entries: Vec::new(),
            children: Vec::new(),
            child_counts: Vec::new(),
            child_entry_counts: Vec::new(),
        }
    }
    fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
    fn subtree_count(&self) -> u64 {
        self.entries.len() as u64 + self.child_counts.iter().sum::<u64>()
    }
}

/// Goal of the destructive descent in `remove_present`.
enum RemoveTarget {
    /// The exact `(key, nonce)` passed to `remove_by_nonce`.
    Exact,
    /// The maximum entry of the current subtree (predecessor extraction
    /// after the target was found in an internal node, CLRS case 2a).
    Max,
    /// The minimum entry of the current subtree (successor extraction,
    /// CLRS case 2b).
    Min,
}

/// Handle to a persistent sorted multimap. Cheap to construct; holds only a
/// namespace. All operations go directly to storage.
pub struct OrderedIndex<K, V, const T: usize = 2> {
    namespace: &'static [u8],
    _marker: PhantomData<(K, V)>,
}

impl<K, V, const T: usize> OrderedIndex<K, V, T> {
    pub const fn new(namespace: &'static [u8]) -> Self {
        assert!(T >= 2, "OrderedIndex: minimum degree T must be >= 2");
        assert!(
            T <= ((u32::MAX as usize) + 1) / 2,
            "OrderedIndex: T too large for mirrored child_entry_counts"
        );
        Self {
            namespace,
            _marker: PhantomData,
        }
    }

    const fn max_keys() -> usize {
        2 * T - 1
    }

    /// SCALE compact-length prefix size for a collection of `value` elements.
    const fn compact_len_bytes(value: usize) -> usize {
        if value < 1 << 6 {
            1
        } else if value < 1 << 14 {
            2
        } else if value < 1 << 30 {
            4
        } else {
            panic!("OrderedIndex: collection length out of compact range")
        }
    }

    /// Exact SCALE-encoded size of a *full internal* node (the worst case),
    /// given upper bounds on the encoded size of one key and one value.
    ///
    /// For a `String`/`Vec<u8>` key of at most `n` bytes the encoded size is
    /// `n` plus its compact length prefix (1 byte for `n < 64`, else 2).
    /// Fixed-width keys encode as their width (`u128` = 16, `[u8; 20]` = 20).
    ///
    /// Every node must fit in a single storage item
    /// ([`MAX_STORAGE_VALUE_BYTES`] = 416 under pallet-revive), which bounds
    /// the per-entry budget by `T`. With a `u32` value and the 8-byte nonce,
    /// the largest encoded key that still fits is roughly:
    ///
    /// | `T` | entries/node | max encoded key bytes |
    /// |-----|--------------|-----------------------|
    /// | 2   | 3            | ~98                   |
    /// | 3   | 5            | ~46                   |
    /// | 4   | 7            | ~24                   |
    ///
    /// Oversized nodes revert at runtime with `OrderedIndexNodeTooLarge`;
    /// use this in a const assertion to reject impossible shapes at build
    /// time instead:
    ///
    /// ```ignore
    /// // u128 key (16 bytes) + 20-byte address value at T = 3:
    /// const _: () = assert!(OrderedIndex::<u128, Address, 3>::fits_storage_limit(16, 20));
    /// ```
    pub const fn max_node_encoded_size(max_key_encoded: usize, max_value_encoded: usize) -> usize {
        let entries = 2 * T - 1;
        let children = 2 * T;
        let entry = max_key_encoded + 8 + max_value_encoded; // key + nonce (u64) + value
        Self::compact_len_bytes(entries)
            + entries * entry
            + Self::compact_len_bytes(children)
            + children * 8 // children: Vec<NodeId>
            + Self::compact_len_bytes(children)
            + children * 8 // child_counts: Vec<u64>
            + Self::compact_len_bytes(children)
            + children * 4 // child_entry_counts: Vec<u32>
    }

    /// True iff a full node fits within [`MAX_STORAGE_VALUE_BYTES`]. Intended
    /// for compile-time shape checks; see [`Self::max_node_encoded_size`].
    pub const fn fits_storage_limit(max_key_encoded: usize, max_value_encoded: usize) -> bool {
        Self::max_node_encoded_size(max_key_encoded, max_value_encoded) <= MAX_STORAGE_VALUE_BYTES
    }

    // --- storage cell accessors -----------------------------------------

    fn root_cell(&self) -> Lazy<NodeId> {
        Lazy::from_key(namespaced_key(self.namespace, &"/root"))
    }
    fn next_id_cell(&self) -> Lazy<u64> {
        Lazy::from_key(namespaced_key(self.namespace, &"/next_id"))
    }
    fn next_nonce_cell(&self) -> Lazy<u64> {
        Lazy::from_key(namespaced_key(self.namespace, &"/next_nonce"))
    }
}

impl<K, V, const T: usize> OrderedIndex<K, V, T>
where
    K: Encode + Decode + Ord + Clone,
    V: Encode + Decode + Clone,
{
    fn nodes(&self) -> Mapping<NodeId, Node<K, V>> {
        Mapping::from_key(namespaced_key(self.namespace, &"/nodes"))
    }

    // --- node-level storage helpers -------------------------------------

    fn load(&self, id: NodeId) -> Node<K, V> {
        self.nodes()
            .get(&id)
            .unwrap_or_else(|| revert(b"OrderedIndexMissingNode"))
    }
    fn store(&self, id: NodeId, node: &Node<K, V>) {
        self.assert_node_shape(node);
        self.assert_node_size(node);
        self.nodes().insert(&id, node);
    }
    fn free(&self, id: NodeId) {
        self.nodes().remove(&id);
    }
    fn alloc(&self, node: &Node<K, V>) -> NodeId {
        let id = self.next_id_cell().get().unwrap_or(1);
        let Some(next) = id.checked_add(1) else {
            revert(b"OrderedIndexNodeIdOverflow");
        };
        self.next_id_cell().set(&next);
        self.store(id, node);
        id
    }
    fn alloc_nonce(&self) -> u64 {
        let n = self.next_nonce_cell().get().unwrap_or(0);
        let Some(next) = n.checked_add(1) else {
            revert(b"OrderedIndexNonceOverflow");
        };
        self.next_nonce_cell().set(&next);
        n
    }
    fn root_id(&self) -> Option<NodeId> {
        match self.root_cell().get() {
            Some(0) | None => None,
            Some(id) => Some(id),
        }
    }
    /// `child_counts[idx] += 1` with overflow check. Used by the insert
    /// descent: the new entry lands somewhere in `children[idx]`'s subtree.
    fn inc_child_count(node: &mut Node<K, V>, idx: usize) {
        let Some(next) = node.child_counts[idx].checked_add(1) else {
            revert(b"OrderedIndexCountOverflow");
        };
        node.child_counts[idx] = next;
    }

    /// `child_counts[idx] -= 1` with underflow check. Used by the remove
    /// descent: exactly one entry disappears from `children[idx]`'s subtree.
    fn dec_child_count(node: &mut Node<K, V>, idx: usize) {
        let Some(next) = node.child_counts[idx].checked_sub(1) else {
            revert(b"OrderedIndexCountUnderflow");
        };
        node.child_counts[idx] = next;
    }

    fn assert_node_shape(&self, node: &Node<K, V>) {
        if node.entries.len() > Self::max_keys() {
            revert(b"OrderedIndexNodeTooManyEntries");
        }

        if node.is_leaf() {
            if !node.child_counts.is_empty() || !node.child_entry_counts.is_empty() {
                revert(b"OrderedIndexLeafHasMirrors");
            }
        } else {
            let expected_children = node.entries.len() + 1;
            if node.children.len() != expected_children
                || node.child_counts.len() != expected_children
                || node.child_entry_counts.len() != expected_children
            {
                revert(b"OrderedIndexBadChildMirrors");
            }
        }

        for i in 1..node.entries.len() {
            let prev = &node.entries[i - 1];
            let curr = &node.entries[i];
            if prev.key > curr.key || (prev.key == curr.key && prev.nonce >= curr.nonce) {
                revert(b"OrderedIndexUnsortedNode");
            }
        }
    }

    fn assert_node_size(&self, node: &Node<K, V>) {
        if node.encoded_size() > MAX_STORAGE_VALUE_BYTES {
            revert(b"OrderedIndexNodeTooLarge");
        }
    }

    // ====================================================================
    // Public API
    // ====================================================================

    /// Total entries in the index.
    pub fn len(&self) -> u64 {
        match self.root_id() {
            None => 0,
            Some(id) => self.load(id).subtree_count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.root_id().is_none()
    }

    /// Insert a new `(k, v)` entry. Duplicate keys are allowed; the new entry
    /// sorts after all existing entries with the same key. Returns the
    /// insertion nonce, which can be passed to `remove_by_nonce` for O(log n)
    /// removal.
    ///
    /// Iterative single-pass preemptive-split descent (stack-safe; see the
    /// module docs). The loop holds at most three nodes in memory: the
    /// pipelined parent frame, the current node, and the child being probed.
    /// A node's mirrors of its descended child (`child_counts` gains the
    /// pending +1, `child_entry_counts` takes the child's final entry count)
    /// are flushed one level behind the descent, once the child's own entry
    /// list can no longer change (i.e. after the child's split-or-not
    /// decision, or after the leaf-level insertion).
    pub fn insert(&self, k: &K, v: &V) -> u64 {
        let nonce = self.alloc_nonce();
        let entry = Entry {
            key: k.clone(),
            nonce,
            value: v.clone(),
        };

        let Some(mut cur_id) = self.root_id() else {
            let mut root = Node::leaf();
            root.entries.push(entry);
            let id = self.alloc(&root);
            self.root_cell().set(&id);
            return nonce;
        };
        let mut cur = self.load(cur_id);

        if cur.entries.len() == Self::max_keys() {
            // Grow: new root, old root as only child, then split that child.
            let mut new_root = Node {
                entries: Vec::new(),
                children: alloc::vec![cur_id],
                child_counts: alloc::vec![cur.subtree_count()],
                child_entry_counts: alloc::vec![cur.entries.len() as u32],
            };
            self.split_child(&mut new_root, 0);
            let new_root_id = self.alloc(&new_root);
            self.root_cell().set(&new_root_id);
            cur_id = new_root_id;
            cur = new_root;
        }

        // Pipelined parent frame: (id, node, index of `cur` in its children).
        // Held in memory until `cur`'s entry list is final, then flushed.
        let mut parent: Option<(NodeId, Node<K, V>, usize)> = None;

        loop {
            // Invariant: `cur` is non-full and in memory.
            if cur.is_leaf() {
                let pos = cur.lower_bound_entry(&entry.key, entry.nonce);
                cur.entries.insert(pos, entry);
                self.store(cur_id, &cur);
                if let Some((parent_id, mut parent_node, cur_idx)) = parent {
                    Self::inc_child_count(&mut parent_node, cur_idx);
                    parent_node.child_entry_counts[cur_idx] = cur.entries.len() as u32;
                    self.store(parent_id, &parent_node);
                }
                return nonce;
            }

            let mut child_idx = cur.lower_bound_entry(&entry.key, entry.nonce);
            let mut child = self.load(cur.children[child_idx]);
            if child.entries.len() == Self::max_keys() {
                self.split_child(&mut cur, child_idx);
                // After split, a new separator sits at cur.entries[child_idx].
                // Descend right if entry sorts after it.
                let sep = &cur.entries[child_idx];
                let goes_right = (entry.key.cmp(&sep.key)).then(entry.nonce.cmp(&sep.nonce))
                    == core::cmp::Ordering::Greater;
                if goes_right {
                    child_idx += 1;
                }
                child = self.load(cur.children[child_idx]);
            }

            // `cur`'s entry list is now final: flush the pipelined parent.
            if let Some((parent_id, mut parent_node, cur_idx)) = parent.take() {
                Self::inc_child_count(&mut parent_node, cur_idx);
                parent_node.child_entry_counts[cur_idx] = cur.entries.len() as u32;
                self.store(parent_id, &parent_node);
            }

            let next_id = cur.children[child_idx];
            parent = Some((cur_id, cur, child_idx));
            cur_id = next_id;
            cur = child;
        }
    }

    /// Find the value of the leftmost (earliest-inserted) entry with key `k`.
    ///
    /// With duplicates allowed, an internal node's `entries[pos]` with key `k`
    /// is *not* necessarily the leftmost - `children[pos]` could hold earlier
    /// duplicates with smaller nonces. So we always descend leftward (into
    /// `children[pos]`) and only commit to the candidate at the leaf.
    pub fn get_first(&self, k: &K) -> Option<V> {
        let mut id = self.root_id()?;
        let mut candidate: Option<V> = None;
        loop {
            let node = self.load(id);
            let pos = node.lower_bound_key(k);
            if pos < node.entries.len() && node.entries[pos].key == *k {
                // Tentative - `children[pos]` may hold an earlier duplicate.
                candidate = Some(node.entries[pos].value.clone());
            }
            if node.is_leaf() {
                return candidate;
            }
            id = node.children[pos];
        }
    }

    /// Does the index contain at least one entry with key `k`?
    pub fn contains_key(&self, k: &K) -> bool {
        self.get_first(k).is_some()
    }

    /// Remove the entry identified by `(k, nonce)` (the nonce returned by
    /// `insert`). Returns the removed value, or `None` if no such entry.
    /// O(log n).
    ///
    /// Two iterative passes (stack-safe; see the module docs): a read-only
    /// existence check first, then — only if the entry is present — a
    /// destructive single-pass descent that is guaranteed to succeed. The
    /// destructive pass decrements the mirrored subtree counts on the way
    /// down, which is only sound because the removal can no longer abort;
    /// the missing-entry path never modifies storage at all.
    pub fn remove_by_nonce(&self, k: &K, nonce: u64) -> Option<V> {
        let root_id = self.root_id()?;
        if !self.contains_entry(root_id, k, nonce) {
            return None;
        }
        Some(self.remove_present(root_id, k, nonce))
    }

    /// Remove the leftmost entry with key `k`, regardless of value.
    /// O(log n).
    pub fn remove_first(&self, k: &K) -> Option<V> {
        let nonce = self.find_first_nonce(k)?;
        self.remove_by_nonce(k, nonce)
    }

    /// Remove the leftmost entry matching both `k` and `v`. Scans duplicate
    /// keys in order; returns `true` on success. O(D * log n) where D is the
    /// number of entries with key `k` (each duplicate inspection re-descends
    /// from the root). For hot paths with heavy duplication, store the nonce
    /// returned by `insert` and call `remove_by_nonce` instead.
    pub fn remove(&self, k: &K, v: &V) -> bool
    where
        V: PartialEq,
    {
        match self.find_nonce_for(k, v) {
            Some(n) => self.remove_by_nonce(k, n).is_some(),
            None => false,
        }
    }

    /// Entry at the given in-order rank (0-based). Returns `(key, value)`.
    /// O(log n).
    pub fn select(&self, mut rank: u64) -> Option<(K, V)> {
        let mut id = self.root_id()?;
        loop {
            let node = self.load(id);
            if rank >= node.subtree_count() {
                return None;
            }
            if node.is_leaf() {
                let e = &node.entries[rank as usize];
                return Some((e.key.clone(), e.value.clone()));
            }
            let mut i = 0;
            loop {
                let c = node.child_counts[i];
                if rank < c {
                    id = node.children[i];
                    break;
                }
                rank -= c;
                if rank == 0 {
                    let e = &node.entries[i];
                    return Some((e.key.clone(), e.value.clone()));
                }
                rank -= 1; // consume the separator entry
                i += 1;
            }
        }
    }

    /// Number of entries strictly before the leftmost entry with key `k`.
    /// If no entry with key `k` exists, returns the count of entries with
    /// keys strictly less than `k` (i.e. the rank where such a key *would*
    /// be inserted). O(log n). Useful for "what page does this key live on?".
    pub fn rank_of_key(&self, k: &K) -> u64 {
        self.rank_of_range_start(Bound::Included(k))
    }

    /// Global rank (0-based) of the first entry satisfying the `from` bound:
    /// the leftmost entry with key `>= k` for `Included(k)`, `> k` for
    /// `Excluded(k)`, rank 0 for `Unbounded`. Equals `len()` when no entry
    /// qualifies. O(log n) - a single root-to-leaf descent summing the
    /// mirrored `child_counts` of the subtrees passed over.
    fn rank_of_range_start(&self, from: Bound<&K>) -> u64 {
        if matches!(from, Bound::Unbounded) {
            return 0;
        }
        let mut id = match self.root_id() {
            Some(id) => id,
            None => return 0,
        };
        let mut rank: u64 = 0;
        loop {
            let node = self.load(id);
            let pos = Self::range_start(&node, from);
            if node.is_leaf() {
                return rank + pos as u64;
            }
            for i in 0..pos {
                rank += node.child_counts[i];
                rank += 1;
            }
            id = node.children[pos];
        }
    }

    /// Collect up to `limit` entries whose keys fall in `[from, to]` (both
    /// bounds honored per `Bound`). Pagination: skip `offset` entries before
    /// starting to collect. O(log n + limit) - the offset is consumed
    /// *positionally*, not by walking.
    ///
    /// Two phases:
    /// 1. **Positional seek.** The global rank of the first entry satisfying
    ///    `from` is computed via the mirrored `child_counts` (one O(log n)
    ///    descent); adding `offset` gives the rank of the first entry to
    ///    yield. A second select-style descent seeks straight to that rank,
    ///    pushing a resume frame per level - so deep offsets cost tree-depth
    ///    reads instead of one read per skipped entry. The in-range entries
    ///    are contiguous in global (key, nonce) order starting at the rank of
    ///    the `from` bound, so an offset that overshoots the range lands
    ///    either past the last entry (caught during the seek) or on an entry
    ///    past `to` (caught by the first emission check); both yield the
    ///    empty result, exactly like skipping entry-by-entry would.
    /// 2. **Streaming.** The classic iterative in-order walk (stack-safe):
    ///    instead of call recursion, an explicit cursor stack of
    ///    `(node id, position)` pairs - two machine words per tree level -
    ///    tracks where to resume in each ancestor. Only one node is decoded
    ///    at a time; an internal node is re-loaded each time the walk
    ///    surfaces back into it (bounded by one extra load per entry yielded
    ///    from internal nodes). The walk stops after `limit` entries or at
    ///    the first entry past `to`, whichever comes first.
    pub fn range(&self, from: Bound<&K>, to: Bound<&K>, offset: u64, limit: u64) -> Vec<(K, V)> {
        let mut out: Vec<(K, V)> = Vec::new();
        if limit == 0 {
            return out;
        }
        let Some(root) = self.root_id() else {
            return out;
        };

        // Rank of the first entry to yield. A `u64` overflow here means the
        // offset is past the end of any possible tree: empty result.
        let Some(target) = self.rank_of_range_start(from).checked_add(offset) else {
            return out;
        };

        // Cursor states: FRESH = first visit. A fresh node was entered by
        // descending past the seek point, so its entire subtree satisfies
        // `from` and the visit starts at position 0. Otherwise, for an
        // internal node `state = pos << 1 | phase` with phase 0 = visit
        // `children[pos]` next, phase 1 = visit `entries[pos]` next; for a
        // leaf, `state` is the entry index to start scanning at (only the
        // seek below produces non-FRESH leaf frames).
        const FRESH: u32 = u32::MAX;
        let mut stack: Vec<(NodeId, u32)> = Vec::new();

        // Seek: descend to the entry at global rank `target`, recording the
        // resume position at every level. Mirrors `select`.
        {
            let mut id = root;
            let mut rank = target;
            'seek: loop {
                let node = self.load(id);
                if rank >= node.subtree_count() {
                    // Only reachable at the root (lower levels satisfy
                    // `rank < subtree_count` by construction): the offset
                    // skips past the last entry of the tree.
                    return out;
                }
                if node.is_leaf() {
                    stack.push((id, rank as u32));
                    break 'seek;
                }
                let mut i = 0;
                loop {
                    let c = node.child_counts[i];
                    if rank < c {
                        // Target is inside children[i]; resume at entries[i]
                        // once that subtree is exhausted.
                        stack.push((id, ((i as u32) << 1) | 1));
                        id = node.children[i];
                        break;
                    }
                    rank -= c;
                    if rank == 0 {
                        // Target is exactly entries[i] of this node.
                        stack.push((id, ((i as u32) << 1) | 1));
                        break 'seek;
                    }
                    rank -= 1; // consume the separator entry
                    i += 1;
                }
            }
        }

        // Stream entries in order from the seeked position.
        while let Some((id, state)) = stack.pop() {
            let node = self.load(id);

            if node.is_leaf() {
                // Fresh leaves sit entirely past the seek point: every entry
                // already satisfies `from`, so the scan starts at 0.
                let start = if state == FRESH { 0 } else { state as usize };
                for e in &node.entries[start..] {
                    // The global in-order walk only grows from here, so the
                    // first entry past `to` ends the whole query.
                    if Self::past_range_end(e, to) {
                        return out;
                    }
                    out.push((e.key.clone(), e.value.clone()));
                    if out.len() as u64 == limit {
                        return out;
                    }
                }
                continue;
            }

            let (pos, visit_child) = if state == FRESH {
                // Fresh internal nodes likewise start at their leftmost child.
                (0, true)
            } else {
                ((state >> 1) as usize, state & 1 == 0)
            };

            if visit_child {
                if pos > node.entries.len() {
                    continue; // defensive; positions run 0..=entries.len()
                }
                // Descend into children[pos]; resume at entries[pos] after.
                stack.push((id, ((pos as u32) << 1) | 1));
                stack.push((node.children[pos], FRESH));
                continue;
            }

            if pos >= node.entries.len() {
                continue; // node exhausted
            }
            let e = &node.entries[pos];
            if Self::past_range_end(e, to) {
                return out;
            }
            out.push((e.key.clone(), e.value.clone()));
            if out.len() as u64 == limit {
                return out;
            }
            stack.push((id, ((pos as u32 + 1) << 1)));
        }
        out
    }

    // ====================================================================
    // Internals: insertion
    // ====================================================================

    /// Split `parent.children[i]`, which must be full, into two `T-1`-entry
    /// siblings with the middle entry promoted to `parent`. Mirrored counts
    /// in `parent` are updated; caller must persist `parent`.
    fn split_child(&self, parent: &mut Node<K, V>, i: usize) {
        let left_id = parent.children[i];
        let mut left = self.load(left_id);

        // Right half of entries past the median; the median itself is promoted.
        let right_entries: Vec<Entry<K, V>> = left.entries.drain(T..).collect();
        let middle = match left.entries.pop() {
            Some(entry) => entry,
            None => revert(b"OrderedIndexMissingMedian"),
        };

        let (right_children, right_child_counts, right_child_entry_counts) = if left.is_leaf() {
            (Vec::new(), Vec::new(), Vec::new())
        } else {
            (
                left.children.drain(T..).collect::<Vec<_>>(),
                left.child_counts.drain(T..).collect::<Vec<_>>(),
                left.child_entry_counts.drain(T..).collect::<Vec<_>>(),
            )
        };

        let right = Node {
            entries: right_entries,
            children: right_children,
            child_counts: right_child_counts,
            child_entry_counts: right_child_entry_counts,
        };

        let left_count = left.subtree_count();
        let left_entry_count = left.entries.len() as u32;
        let right_count = right.subtree_count();
        let right_entry_count = right.entries.len() as u32;

        self.store(left_id, &left);
        let right_id = self.alloc(&right);

        parent.entries.insert(i, middle);
        parent.children.insert(i + 1, right_id);
        parent.child_counts[i] = left_count;
        parent.child_counts.insert(i + 1, right_count);
        parent.child_entry_counts[i] = left_entry_count;
        parent.child_entry_counts.insert(i + 1, right_entry_count);
    }

    // ====================================================================
    // Internals: removal
    // ====================================================================

    /// Read-only descent: does the subtree rooted at `root_id` contain an
    /// entry with exactly this `(key, nonce)`? O(log n), no writes.
    fn contains_entry(&self, root_id: NodeId, k: &K, nonce: u64) -> bool {
        let mut id = root_id;
        loop {
            let node = self.load(id);
            let pos = node.lower_bound_entry(k, nonce);
            if pos < node.entries.len()
                && node.entries[pos].key == *k
                && node.entries[pos].nonce == nonce
            {
                return true;
            }
            if node.is_leaf() {
                return false;
            }
            id = node.children[pos];
        }
    }

    /// Destructive single-pass removal of `(k, nonce)`, which the caller has
    /// verified to exist. Iterative CLRS 18.3 descent:
    ///
    /// - Every node entered (other than the root) is guaranteed at least `T`
    ///   entries by `descend_prepared` (borrow or merge on the way down).
    /// - Case 1 (target in a leaf): remove it there.
    /// - Case 2a/2b (target in an internal node, with a `>= T`-entry child on
    ///   the relevant side): record the slot in `swap` and switch the descent
    ///   target to the predecessor (`Max` of the left subtree) or successor
    ///   (`Min` of the right subtree). The extracted leaf entry is patched
    ///   into the recorded slot at the end; this is the only deferred write
    ///   and it never changes the slot node's entry count, so the count
    ///   mirrors flushed during the descent stay exact.
    /// - Case 2c (both neighbours min-filled): merge them, pulling the target
    ///   down, and keep descending for it.
    ///
    /// Like `insert`, the parent frame is pipelined one level behind the
    /// descent and flushed (with `child_counts -= 1` and the child's final
    /// entry count) once the current node's entry list is final. An empty
    /// root (leaf or internal) is collapsed on the spot, matching the
    /// previous recursive implementation's on-disk results.
    fn remove_present(&self, root_id: NodeId, k: &K, nonce: u64) -> V {
        let mut target = RemoveTarget::Exact;
        // Slot awaiting its predecessor/successor: (node id, entry index).
        let mut swap: Option<(NodeId, usize)> = None;
        // Value of the original `(k, nonce)` entry once seen in an internal
        // node; the return value when the leaf extraction feeds a swap.
        let mut found_value: Option<V> = None;
        // Pipelined parent frame: (id, node, index of `cur` in its children).
        let mut parent: Option<(NodeId, Node<K, V>, usize)> = None;
        let mut cur_id = root_id;
        let mut cur = self.load(cur_id);

        loop {
            if cur.is_leaf() {
                let idx = match target {
                    RemoveTarget::Exact => {
                        let pos = cur.lower_bound_entry(k, nonce);
                        let found = pos < cur.entries.len()
                            && cur.entries[pos].key == *k
                            && cur.entries[pos].nonce == nonce;
                        if !found {
                            // Unreachable if `contains_entry` said yes.
                            revert(b"OrderedIndexRemoveLostEntry");
                        }
                        pos
                    }
                    RemoveTarget::Max => match cur.entries.len().checked_sub(1) {
                        Some(last) => last,
                        None => revert(b"OrderedIndexEmptyMaxLeaf"),
                    },
                    RemoveTarget::Min => {
                        if cur.entries.is_empty() {
                            revert(b"OrderedIndexEmptyMinLeaf");
                        }
                        0
                    }
                };
                let extracted = cur.entries.remove(idx);

                if parent.is_none() && cur.entries.is_empty() {
                    // The root was this leaf's last holder; drop the tree.
                    self.free(cur_id);
                    self.root_cell().set(&0);
                } else {
                    self.store(cur_id, &cur);
                }
                if let Some((parent_id, mut parent_node, cur_idx)) = parent {
                    Self::dec_child_count(&mut parent_node, cur_idx);
                    parent_node.child_entry_counts[cur_idx] = cur.entries.len() as u32;
                    self.store(parent_id, &parent_node);
                }

                return match swap {
                    None => extracted.value,
                    Some((swap_id, swap_pos)) => {
                        // Patch the predecessor/successor into the slot the
                        // target was found in. Loaded fresh: the pipeline has
                        // already flushed every count adjustment to it.
                        let mut swap_node = self.load(swap_id);
                        swap_node.entries[swap_pos] = extracted;
                        self.store(swap_id, &swap_node);
                        match found_value {
                            Some(value) => value,
                            None => revert(b"OrderedIndexRemoveLostValue"),
                        }
                    }
                };
            }

            // Internal node: pick the child to descend into, rebalancing or
            // merging on the way down so it can absorb a removal.
            let descend_idx = match target {
                RemoveTarget::Exact => {
                    let pos = cur.lower_bound_entry(k, nonce);
                    let found = pos < cur.entries.len()
                        && cur.entries[pos].key == *k
                        && cur.entries[pos].nonce == nonce;
                    if found {
                        // The thresholds are on the child's *own* entry count,
                        // not its subtree size - a min-filled internal child
                        // can have a huge subtree, but stealing from it would
                        // still violate the B-tree minimum-key invariant.
                        found_value = Some(cur.entries[pos].value.clone());
                        if cur.child_entry_counts[pos] >= T as u32 {
                            // 2a: extract predecessor from fat left child.
                            swap = Some((cur_id, pos));
                            target = RemoveTarget::Max;
                            pos
                        } else if cur.child_entry_counts[pos + 1] >= T as u32 {
                            // 2b: extract successor from fat right child.
                            swap = Some((cur_id, pos));
                            target = RemoveTarget::Min;
                            pos + 1
                        } else {
                            // 2c: both children are minimum-filled; merge them
                            // (pulling the target down) and keep descending.
                            self.merge_children(&mut cur, pos);
                            pos
                        }
                    } else {
                        self.descend_prepared(&mut cur, pos)
                    }
                }
                RemoveTarget::Max => {
                    let last = cur.entries.len();
                    self.descend_prepared(&mut cur, last)
                }
                RemoveTarget::Min => self.descend_prepared(&mut cur, 0),
            };

            // A merge above may have emptied an internal root (it had one
            // entry and its two children were fused). Promote the merged
            // child and restart this level; no counts to adjust since the
            // root has no parent mirror.
            if parent.is_none() && cur.entries.is_empty() {
                let new_root_id = cur.children[0];
                self.free(cur_id);
                self.root_cell().set(&new_root_id);
                cur_id = new_root_id;
                cur = self.load(cur_id);
                continue;
            }

            // `cur`'s entry list is now final: flush the pipelined parent.
            if let Some((parent_id, mut parent_node, cur_idx)) = parent.take() {
                Self::dec_child_count(&mut parent_node, cur_idx);
                parent_node.child_entry_counts[cur_idx] = cur.entries.len() as u32;
                self.store(parent_id, &parent_node);
            }

            let next_id = cur.children[descend_idx];
            parent = Some((cur_id, cur, descend_idx));
            cur_id = next_id;
            cur = self.load(cur_id);
        }
    }

    /// Ensure `node.children[pos]` has at least `T` entries (so that a
    /// downstream remove will not underflow it). Either borrows from a
    /// sibling or merges. Returns the (possibly shifted) child index to
    /// descend into.
    fn descend_prepared(&self, node: &mut Node<K, V>, pos: usize) -> usize {
        // Threshold is on the child's own entry count (`child_entry_counts`),
        // never on its subtree size. An internal child with `T-1` entries can
        // still have a large subtree, but it's still min-filled and unsafe
        // to remove from without rebalancing.
        if node.child_entry_counts[pos] >= T as u32 {
            return pos;
        }
        let has_left = pos > 0;
        let has_right = pos + 1 < node.children.len();

        if has_left && node.child_entry_counts[pos - 1] >= T as u32 {
            self.borrow_from_left(node, pos);
            pos
        } else if has_right && node.child_entry_counts[pos + 1] >= T as u32 {
            self.borrow_from_right(node, pos);
            pos
        } else if has_right {
            self.merge_children(node, pos);
            pos
        } else {
            // has_left must be true (node has >= 1 child, and !has_right).
            self.merge_children(node, pos - 1);
            pos - 1
        }
    }

    /// Rotate one entry out of the left sibling, through the parent
    /// separator, into the front of `children[pos]`.
    fn borrow_from_left(&self, node: &mut Node<K, V>, pos: usize) {
        let left_id = node.children[pos - 1];
        let child_id = node.children[pos];
        let mut left = self.load(left_id);
        let mut child = self.load(child_id);

        let separator = node.entries[pos - 1].clone();
        let new_separator = match left.entries.pop() {
            Some(entry) => entry,
            None => revert(b"OrderedIndexBorrowLeftEmpty"),
        };
        child.entries.insert(0, separator);
        node.entries[pos - 1] = new_separator;

        if !left.is_leaf() {
            let moved_child = match left.children.pop() {
                Some(value) => value,
                None => revert(b"OrderedIndexMissingLeftChild"),
            };
            let moved_count = match left.child_counts.pop() {
                Some(value) => value,
                None => revert(b"OrderedIndexMissingLeftCount"),
            };
            let moved_entry_count = match left.child_entry_counts.pop() {
                Some(value) => value,
                None => revert(b"OrderedIndexMissingLeftEntryCount"),
            };
            child.children.insert(0, moved_child);
            child.child_counts.insert(0, moved_count);
            child.child_entry_counts.insert(0, moved_entry_count);
        }

        node.child_counts[pos - 1] = left.subtree_count();
        node.child_counts[pos] = child.subtree_count();
        node.child_entry_counts[pos - 1] = left.entries.len() as u32;
        node.child_entry_counts[pos] = child.entries.len() as u32;

        self.store(left_id, &left);
        self.store(child_id, &child);
    }

    /// Mirror of `borrow_from_left`.
    fn borrow_from_right(&self, node: &mut Node<K, V>, pos: usize) {
        let child_id = node.children[pos];
        let right_id = node.children[pos + 1];
        let mut child = self.load(child_id);
        let mut right = self.load(right_id);

        let separator = node.entries[pos].clone();
        let new_separator = right.entries.remove(0);
        child.entries.push(separator);
        node.entries[pos] = new_separator;

        if !right.is_leaf() {
            let moved_child = right.children.remove(0);
            let moved_count = right.child_counts.remove(0);
            let moved_entry_count = right.child_entry_counts.remove(0);
            child.children.push(moved_child);
            child.child_counts.push(moved_count);
            child.child_entry_counts.push(moved_entry_count);
        }

        node.child_counts[pos] = child.subtree_count();
        node.child_counts[pos + 1] = right.subtree_count();
        node.child_entry_counts[pos] = child.entries.len() as u32;
        node.child_entry_counts[pos + 1] = right.entries.len() as u32;

        self.store(child_id, &child);
        self.store(right_id, &right);
    }

    /// Merge `children[pos]` and `children[pos + 1]` into `children[pos]`,
    /// pulling down `entries[pos]` as the separator between them. Frees the
    /// right child and updates the parent's mirrored structures.
    fn merge_children(&self, node: &mut Node<K, V>, pos: usize) {
        let left_id = node.children[pos];
        let right_id = node.children[pos + 1];
        let mut left = self.load(left_id);
        let right = self.load(right_id);

        let separator = node.entries.remove(pos);
        left.entries.push(separator);
        left.entries.extend(right.entries);
        if !left.is_leaf() {
            left.children.extend(right.children);
            left.child_counts.extend(right.child_counts);
            left.child_entry_counts.extend(right.child_entry_counts);
        }

        node.children.remove(pos + 1);
        node.child_counts.remove(pos + 1);
        node.child_entry_counts.remove(pos + 1);
        node.child_counts[pos] = left.subtree_count();
        node.child_entry_counts[pos] = left.entries.len() as u32;

        self.store(left_id, &left);
        self.free(right_id);
    }

    // --- nonce lookup for value-keyed removes --------------------------

    /// Same descent as `get_first`: track a tentative match at every level
    /// and only commit at the leaf, since `children[pos]` may hold an earlier
    /// duplicate (smaller nonce, same key).
    fn find_first_nonce(&self, k: &K) -> Option<u64> {
        let mut id = self.root_id()?;
        let mut candidate: Option<u64> = None;
        loop {
            let node = self.load(id);
            let pos = node.lower_bound_key(k);
            if pos < node.entries.len() && node.entries[pos].key == *k {
                candidate = Some(node.entries[pos].nonce);
            }
            if node.is_leaf() {
                return candidate;
            }
            id = node.children[pos];
        }
    }

    fn find_nonce_for(&self, k: &K, v: &V) -> Option<u64>
    where
        V: PartialEq,
    {
        // Locate the first entry with key == k, then walk the sorted order
        // forward until we see a different key.
        let rank = self.rank_of_key(k);
        let total = self.len();
        let mut cursor = rank;
        while cursor < total {
            let (ck, nonce, cv) = self.select_with_nonce(cursor)?;
            if ck != *k {
                return None;
            }
            if cv == *v {
                return Some(nonce);
            }
            cursor += 1;
        }
        None
    }

    /// Like `select`, but also returns the internal nonce. Private.
    fn select_with_nonce(&self, mut rank: u64) -> Option<(K, u64, V)> {
        let mut id = self.root_id()?;
        loop {
            let node = self.load(id);
            if rank >= node.subtree_count() {
                return None;
            }
            if node.is_leaf() {
                let e = &node.entries[rank as usize];
                return Some((e.key.clone(), e.nonce, e.value.clone()));
            }
            let mut i = 0;
            loop {
                let c = node.child_counts[i];
                if rank < c {
                    id = node.children[i];
                    break;
                }
                rank -= c;
                if rank == 0 {
                    let e = &node.entries[i];
                    return Some((e.key.clone(), e.nonce, e.value.clone()));
                }
                rank -= 1;
                i += 1;
            }
        }
    }

    // --- range iteration helpers ----------------------------------------

    /// First position in `node` whose entry (and the child to its left) can
    /// still intersect the `from` bound.
    fn range_start(node: &Node<K, V>, from: Bound<&K>) -> usize {
        match from {
            Bound::Unbounded => 0,
            Bound::Included(k) => node.lower_bound_key(k),
            Bound::Excluded(k) => node.upper_bound_key(k),
        }
    }

    /// Is `e` past the `to` bound? Since the walk is in-order, the first
    /// entry past the bound terminates the whole query.
    fn past_range_end(e: &Entry<K, V>, to: Bound<&K>) -> bool {
        match to {
            Bound::Unbounded => false,
            Bound::Included(k) => e.key > *k,
            Bound::Excluded(k) => e.key >= *k,
        }
    }
}

#[cfg(test)]
mod tests;

// ========================================================================
// Node-level search helpers - inherent methods so `K` and `V` stay in scope.
// ========================================================================

impl<K: Ord, V> Node<K, V> {
    /// Index of the first entry whose key is `>= k`. Within a run of equal
    /// keys this returns the leftmost one.
    fn lower_bound_key(&self, k: &K) -> usize {
        self.entries.partition_point(|e| e.key < *k)
    }

    /// Index of the first entry whose key is `> k`.
    fn upper_bound_key(&self, k: &K) -> usize {
        self.entries.partition_point(|e| e.key <= *k)
    }

    /// Index of the first entry whose `(key, nonce)` is `>= (k, nonce)`.
    /// Used to locate an exact internal entry for removal.
    fn lower_bound_entry(&self, k: &K, nonce: u64) -> usize {
        self.entries
            .partition_point(|e| e.key < *k || (e.key == *k && e.nonce < nonce))
    }
}
