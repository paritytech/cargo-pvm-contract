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
//! - `range(...)`: O(log n + items_scanned).

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
/// - `child_counts[i]` mirrors `children[i].subtree_count()` (used by rank/select).
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
    fn refresh_child_mirrors(&self, node: &mut Node<K, V>, child_idx: usize) {
        let child = self.load(node.children[child_idx]);
        node.child_counts[child_idx] = child.subtree_count();
        node.child_entry_counts[child_idx] = child.entries.len() as u32;
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
    pub fn insert(&self, k: &K, v: &V) -> u64 {
        let nonce = self.alloc_nonce();
        let entry = Entry {
            key: k.clone(),
            nonce,
            value: v.clone(),
        };

        match self.root_id() {
            None => {
                let mut root = Node::leaf();
                root.entries.push(entry);
                let id = self.alloc(&root);
                self.root_cell().set(&id);
            }
            Some(root_id) => {
                let root = self.load(root_id);
                if root.entries.len() == Self::max_keys() {
                    // Grow: new root, old root as only child, then split that child.
                    let mut new_root = Node {
                        entries: Vec::new(),
                        children: alloc::vec![root_id],
                        child_counts: alloc::vec![root.subtree_count()],
                        child_entry_counts: alloc::vec![root.entries.len() as u32],
                    };
                    self.split_child(&mut new_root, 0);
                    let new_root_id = self.alloc(&new_root);
                    self.root_cell().set(&new_root_id);
                    self.insert_nonfull(new_root_id, entry);
                } else {
                    self.insert_nonfull(root_id, entry);
                }
            }
        }
        nonce
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
    pub fn remove_by_nonce(&self, k: &K, nonce: u64) -> Option<V> {
        let root_id = self.root_id()?;
        let removed = self.remove_from(root_id, k, nonce);
        // Collapse an empty root: if the root is an internal node with no
        // entries left (can happen after a Case-2c merge), its sole child
        // becomes the new root.
        if removed.is_some() {
            let root = self.load(root_id);
            if root.entries.is_empty() {
                if root.is_leaf() {
                    self.free(root_id);
                    self.root_cell().set(&0);
                } else {
                    let new_root = root.children[0];
                    self.free(root_id);
                    self.root_cell().set(&new_root);
                }
            }
        }
        removed
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
        let mut id = match self.root_id() {
            Some(id) => id,
            None => return 0,
        };
        let mut rank: u64 = 0;
        loop {
            let node = self.load(id);
            let pos = node.lower_bound_key(k);
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
    /// starting to collect. O(log n + items scanned).
    pub fn range(&self, from: Bound<&K>, to: Bound<&K>, offset: u64, limit: u64) -> Vec<(K, V)> {
        let mut out: Vec<(K, V)> = Vec::new();
        if limit == 0 {
            return out;
        }
        let mut skipped: u64 = 0;
        if let Some(root) = self.root_id() {
            self.range_walk(root, from, to, offset, limit, &mut skipped, &mut out);
        }
        out
    }

    // ====================================================================
    // Internals: insertion
    // ====================================================================

    /// Insert into a subtree whose root is guaranteed to be non-full. All
    /// nodes on the downward path are preemptively split if full, so we
    /// never need to walk back up.
    fn insert_nonfull(&self, node_id: NodeId, entry: Entry<K, V>) {
        let mut node = self.load(node_id);
        let pos = node.lower_bound_entry(&entry.key, entry.nonce);

        if node.is_leaf() {
            node.entries.insert(pos, entry);
            self.store(node_id, &node);
            return;
        }

        // Load the child so we can preemptively split if it is full before
        // descending. We do not try to predict how the child's mirrored
        // counts will change; insertion can cause deeper splits that increase
        // the child's own `entries.len()`, so we refresh the exact mirrors
        // after recursion instead.
        let mut child_idx = pos;
        let child = self.load(node.children[child_idx]);
        if child.entries.len() == Self::max_keys() {
            self.split_child(&mut node, child_idx);
            // After split, a new separator sits at node.entries[child_idx].
            // Descend right if entry sorts after it.
            let sep = &node.entries[child_idx];
            let goes_right = (entry.key.cmp(&sep.key)).then(entry.nonce.cmp(&sep.nonce))
                == core::cmp::Ordering::Greater;
            if goes_right {
                child_idx += 1;
            }
        }

        let descend = node.children[child_idx];
        self.insert_nonfull(descend, entry);
        self.refresh_child_mirrors(&mut node, child_idx);
        self.store(node_id, &node);
    }

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

    /// Remove the entry `(k, nonce)` from the subtree rooted at `node_id`.
    /// On entry, `node_id` is either the tree root or a node with at least
    /// `T` entries (the rebalancing in `descend_prepared` maintains this).
    fn remove_from(&self, node_id: NodeId, k: &K, nonce: u64) -> Option<V> {
        let mut node = self.load(node_id);
        let pos = node.lower_bound_entry(k, nonce);
        let at_pos = pos < node.entries.len()
            && node.entries[pos].key == *k
            && node.entries[pos].nonce == nonce;

        if at_pos {
            if node.is_leaf() {
                // Case 1: in a leaf - just remove.
                let removed = node.entries.remove(pos).value;
                self.store(node_id, &node);
                return Some(removed);
            }
            // Case 2: in an internal node. Cases 2a / 2b / 2c from CLRS 18.3.
            // The check has to be on the child's *own* entry count, not its
            // subtree size - a min-filled internal child can have a huge
            // subtree, but stealing from it would still violate the B-tree
            // minimum-key invariant.
            let original = node.entries[pos].value.clone();

            if node.child_entry_counts[pos] >= T as u32 {
                // 2a: steal predecessor from fat left child.
                let pred = self.find_max(node.children[pos]);
                self.remove_from(node.children[pos], &pred.key, pred.nonce);
                node.entries[pos] = pred;
                self.refresh_child_mirrors(&mut node, pos);
                self.store(node_id, &node);
            } else if node.child_entry_counts[pos + 1] >= T as u32 {
                // 2b: steal successor from fat right child.
                let succ = self.find_min(node.children[pos + 1]);
                self.remove_from(node.children[pos + 1], &succ.key, succ.nonce);
                node.entries[pos] = succ;
                self.refresh_child_mirrors(&mut node, pos + 1);
                self.store(node_id, &node);
            } else {
                // 2c: both children are minimum-filled; merge them (pulling
                // the separator down) into one 2T-1-entry node, then recurse.
                self.merge_children(&mut node, pos);
                let merged_id = node.children[pos];
                self.store(node_id, &node);
                let _ = self.remove_from(merged_id, k, nonce);
                // Refresh the mirrored counts after recursion.
                self.refresh_child_mirrors(&mut node, pos);
                self.store(node_id, &node);
            }
            return Some(original);
        }

        // Case 3: not in this node - must descend.
        if node.is_leaf() {
            return None;
        }
        let descend = self.descend_prepared(&mut node, pos);
        let child_id = node.children[descend];
        self.store(node_id, &node);
        let result = self.remove_from(child_id, k, nonce);
        if result.is_some() {
            // Refresh the mirrored counts for the subtree we touched.
            self.refresh_child_mirrors(&mut node, descend);
            self.store(node_id, &node);
        }
        result
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

    fn find_max(&self, mut id: NodeId) -> Entry<K, V> {
        loop {
            let node = self.load(id);
            if node.is_leaf() {
                return match node.entries.last() {
                    Some(entry) => entry.clone(),
                    None => revert(b"OrderedIndexEmptyMaxLeaf"),
                };
            }
            id = match node.children.last() {
                Some(child_id) => *child_id,
                None => revert(b"OrderedIndexMissingMaxChild"),
            };
        }
    }
    fn find_min(&self, mut id: NodeId) -> Entry<K, V> {
        loop {
            let node = self.load(id);
            if node.is_leaf() {
                return match node.entries.first() {
                    Some(entry) => entry.clone(),
                    None => revert(b"OrderedIndexEmptyMinLeaf"),
                };
            }
            id = node.children[0];
        }
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

    // --- range iteration (in-order walk with early termination) ---------

    fn range_walk(
        &self,
        node_id: NodeId,
        from: Bound<&K>,
        to: Bound<&K>,
        offset: u64,
        limit: u64,
        skipped: &mut u64,
        out: &mut Vec<(K, V)>,
    ) -> bool {
        if out.len() as u64 == limit {
            return true; // stop
        }
        let node = self.load(node_id);

        // Compute the range of entry indices whose subtrees / values can
        // possibly match. For keys strictly below `from`, we skip both the
        // entry and its left subtree; for keys above `to`, we stop early.
        let start = match from {
            Bound::Unbounded => 0,
            Bound::Included(k) => node.lower_bound_key(k),
            Bound::Excluded(k) => node.upper_bound_key(k),
        };

        for i in start..=node.entries.len() {
            // Left child of entries[i] (or right child of entries[i-1]).
            if !node.is_leaf() {
                if self.range_walk(node.children[i], from, to, offset, limit, skipped, out) {
                    return true;
                }
                if out.len() as u64 == limit {
                    return true;
                }
            }
            if i == node.entries.len() {
                break;
            }

            let e = &node.entries[i];
            let above = match to {
                Bound::Unbounded => false,
                Bound::Included(k) => e.key > *k,
                Bound::Excluded(k) => e.key >= *k,
            };
            if above {
                return true;
            }
            let below = match from {
                Bound::Unbounded => false,
                Bound::Included(k) => e.key < *k,
                Bound::Excluded(k) => e.key <= *k,
            };
            if below {
                continue;
            }

            if *skipped < offset {
                *skipped += 1;
            } else {
                out.push((e.key.clone(), e.value.clone()));
                if out.len() as u64 == limit {
                    return true;
                }
            }
        }
        false
    }
}

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
