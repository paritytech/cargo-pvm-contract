extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;
use core::ops::Bound;

use pvm_contract_types::{DecodeError, Host, SolDecode, SolEncode};

use crate::{
    storage_clear_value, storage_get_bytes, storage_set_bytes, AsStorageKey, Lazy, StorageKey,
    MAX_STORAGE_VALUE_BYTES,
};

/// Compact raw-byte body encoding for the OrderedIndex B-tree node's
/// per-entry K and V. Replaces the 32-byte-aligned `SolEncode` ABI body
/// (a `String` of N UTF-8 bytes is N here vs 32+N+pad there), which lifts
/// the B-tree degree from T=2 (forced by the 416-byte per-slot cap) to
/// T=4..8 and roughly halves slot-reads per range query.
///
/// `compact_encoded_len` returns the **body** byte count, not the on-wire
/// length: the OrderedIndex node writes its own 2-byte big-endian `k_len`
/// / `v_len` header in front of the body. Callers that need a length
/// prefix in a different framing must add it themselves.
pub trait CompactCodec: Sized {
    /// Number of body bytes `compact_encode_to` will write.
    fn compact_encoded_len(&self) -> usize;

    /// Write the body bytes into `out`, advancing the cursor.
    fn compact_encode_to<'a>(&self, out: &'a mut &'a mut [u8]);

    /// Read the body bytes from `input`, advancing the cursor. Fails
    /// closed on truncation or invalid bytes — never panics on bad input.
    fn compact_decode_from(input: &mut &[u8]) -> Result<Self, DecodeError>;
}

impl CompactCodec for String {
    fn compact_encoded_len(&self) -> usize {
        self.len()
    }

    fn compact_encode_to<'a>(&self, out: &'a mut &'a mut [u8]) {
        let n = self.len();
        assert!(
            n <= out.len(),
            "CompactCodec<String>::compact_encode_to: buffer too small (need {}, have {})",
            n,
            out.len(),
        );
        out[..n].copy_from_slice(self.as_bytes());
        let (_, rest) = out.split_at_mut(n);
        *out = rest;
    }

    fn compact_decode_from(input: &mut &[u8]) -> Result<Self, DecodeError> {
        let s = String::from_utf8(input.to_vec()).map_err(|_| DecodeError)?;
        *input = &[];
        Ok(s)
    }
}

impl CompactCodec for u64 {
    fn compact_encoded_len(&self) -> usize {
        let mut v = *self;
        let mut len = 1;
        while v >= 0x80 {
            len += 1;
            v >>= 7;
        }
        len
    }

    fn compact_encode_to<'a>(&self, out: &'a mut &'a mut [u8]) {
        let mut v = *self;
        loop {
            assert!(
                !out.is_empty(),
                "CompactCodec<u64>::compact_encode_to: buffer too small",
            );
            let byte = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out[0] = byte;
                let (_, rest) = out.split_at_mut(1);
                *out = rest;
                return;
            }
            out[0] = byte | 0x80;
            let (_, rest) = out.split_at_mut(1);
            *out = rest;
        }
    }

    fn compact_decode_from(input: &mut &[u8]) -> Result<Self, DecodeError> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            if input.is_empty() {
                return Err(DecodeError);
            }
            let byte = input[0];
            *input = &input[1..];
            result |= u64::from(byte & 0x7F).wrapping_shl(shift);
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift >= 70 {
                return Err(DecodeError);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(pub u64);

impl AsStorageKey for NodeId {
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
        self.0.derive_slot(host, root)
    }
}

struct Entry<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
> {
    key: K,
    nonce: u64,
    value: V,
}

impl<
        K: SolEncode + SolDecode + Clone + CompactCodec,
        V: SolEncode + SolDecode + Clone + CompactCodec,
    > Clone for Entry<K, V>
{
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            nonce: self.nonce,
            value: self.value.clone(),
        }
    }
}

struct Node<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
> {
    entries: Vec<Entry<K, V>>,
    children: Vec<NodeId>,
    child_counts: Vec<u64>,
    child_entry_counts: Vec<u32>,
}

impl<
        K: SolEncode + SolDecode + Clone + CompactCodec,
        V: SolEncode + SolDecode + Clone + CompactCodec,
    > Clone for Node<K, V>
{
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            children: self.children.clone(),
            child_counts: self.child_counts.clone(),
            child_entry_counts: self.child_entry_counts.clone(),
        }
    }
}

impl<
        K: SolEncode + SolDecode + Clone + CompactCodec,
        V: SolEncode + SolDecode + Clone + CompactCodec,
    > Node<K, V>
{
    fn encode_v(&self, v: &V, out: &mut Vec<u8>) {
        let body_len = v.compact_encoded_len();
        let start = out.len();
        out.resize(start + body_len, 0);
        let mut cursor: &mut [u8] = &mut out[start..start + body_len];
        v.compact_encode_to(&mut cursor);
    }

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

    // On-wire node layout with per-node key prefix compression (BE throughout,
    // capacities enforced by the `T <= 128` assert in `new`):
    //   header     : 1B flags | 1B entries_len | 1B children_len | 1B prefix_len = 4B
    //   prefix     : [prefix_len bytes] — common prefix shared by ALL entry keys in this node
    //   per entry  : 8B nonce | 1B k_suffix_len | k_suffix | 1B v_len | v_body
    //   per child  : 4B node_id | 4B subtree_count | 1B own_entry_count = 9B
    //
    // The common prefix of sorted keys equals the common prefix of the FIRST and
    // LAST key only — O(1) to compute. Each entry stores only the key suffix
    // (key minus the shared prefix), so sorted username corpora
    // ("user0000".."user9999") pay ~4-7B prefix once per node and ~3-6B suffix
    // per entry instead of ~10B for the full key, enabling higher T (fanout).
    fn encode(&self) -> Vec<u8> {
        // Prefix compression only helps with 2+ entries; for 0-1 entries it
        // adds header overhead with zero savings.
        let prefix: Vec<u8> = match (self.entries.first(), self.entries.last()) {
            (Some(first), Some(last)) if self.entries.len() >= 2 => {
                let fb = encode_codec_bytes(&first.key);
                let lb = encode_codec_bytes(&last.key);
                let plen = fb
                    .iter()
                    .zip(lb.iter())
                    .take_while(|(a, b)| a == b)
                    .count()
                    .min(255);
                fb[..plen].to_vec()
            }
            _ => Vec::new(),
        };
        let prefix_len = prefix.len();

        let mut entries_byte_len = 0usize;
        for e in &self.entries {
            let suffix_len = e.key.compact_encoded_len().saturating_sub(prefix_len);
            entries_byte_len = entries_byte_len
                .checked_add(e.nonce.compact_encoded_len())
                .and_then(|n| n.checked_add(1)) // k_suffix_len
                .and_then(|n| n.checked_add(suffix_len)) // k_suffix body
                .and_then(|n| n.checked_add(1)) // v_len
                .and_then(|n| n.checked_add(e.value.compact_encoded_len())) // v body
                .expect("Node::encode: entry sizes overflow");
        }
        let header_len = 1usize + 1 + 1 + 1; // flags + entries_len + children_len + prefix_len
        let entries_len_field =
            u8::try_from(self.entries.len()).expect("Node::encode: entries_len > 255 (T>128)");
        let children_len_field =
            u8::try_from(self.children.len()).expect("Node::encode: children_len > 255 (T>128)");
        let children_byte_len: usize = (0..self.children.len())
            .map(|i| {
                self.children[i].0.compact_encoded_len()
                    + self.child_counts[i].compact_encoded_len()
                    + 1
            })
            .sum();
        let total = header_len
            .checked_add(prefix_len)
            .and_then(|n| n.checked_add(entries_byte_len))
            .and_then(|n| n.checked_add(children_byte_len))
            .expect("Node::encode: total size overflow");
        assert!(
            total <= MAX_STORAGE_VALUE_BYTES,
            "Node::encode: {} bytes exceeds MAX_STORAGE_VALUE_BYTES ({})",
            total,
            MAX_STORAGE_VALUE_BYTES,
        );

        let mut out = Vec::with_capacity(total);
        let flags: u8 = if self.is_leaf() { 0x01 } else { 0x00 };
        out.push(flags);
        out.push(entries_len_field);
        out.push(children_len_field);
        out.push(u8::try_from(prefix_len).expect("prefix_len <= 255 (capped above)"));
        out.extend_from_slice(&prefix);

        for e in &self.entries {
            let nonce_len = e.nonce.compact_encoded_len();
            let start = out.len();
            out.resize(start + nonce_len, 0);
            let mut cursor: &mut [u8] = &mut out[start..start + nonce_len];
            e.nonce.compact_encode_to(&mut cursor);
            let key_bytes = encode_codec_bytes(&e.key);
            let suffix = &key_bytes[prefix_len..];
            let k_suffix_len = u8::try_from(suffix.len())
                .expect("Node::encode: key suffix > 255 bytes");
            out.push(k_suffix_len);
            out.extend_from_slice(suffix);
            let v_len = u8::try_from(e.value.compact_encoded_len())
                .expect("Node::encode: value body > 255 bytes");
            out.push(v_len);
            self.encode_v(&e.value, &mut out);
        }
        for i in 0..self.children.len() {
            let nid = self.children[i].0;
            let nid_len = nid.compact_encoded_len();
            let start = out.len();
            out.resize(start + nid_len, 0);
            let mut cursor: &mut [u8] = &mut out[start..start + nid_len];
            nid.compact_encode_to(&mut cursor);
            let sc = self.child_counts[i];
            let sc_len = sc.compact_encoded_len();
            let start = out.len();
            out.resize(start + sc_len, 0);
            let mut cursor: &mut [u8] = &mut out[start..start + sc_len];
            sc.compact_encode_to(&mut cursor);
            out.push(u8::try_from(self.child_entry_counts[i])
                .expect("Node::encode: own_entry_count > 255 (T>128)"));
        }
        out
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        const HEADER_LEN: usize = 1 + 1 + 1 + 1; // flags + entries_len + children_len + prefix_len
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let flags = bytes[0];
        let is_leaf = (flags & 0x01) != 0;
        let entries_len = bytes[1] as usize;
        let children_len = bytes[2] as usize;
        let prefix_len = bytes[3] as usize;
        if is_leaf {
            if children_len != 0 {
                return None;
            }
        } else if children_len != entries_len + 1 {
            return None;
        }

        let mut cursor = HEADER_LEN;
        if cursor.checked_add(prefix_len)? > bytes.len() {
            return None;
        }
        let prefix = &bytes[cursor..cursor + prefix_len];
        cursor += prefix_len;

        let mut entries: Vec<Entry<K, V>> = Vec::with_capacity(entries_len);
        for _ in 0..entries_len {
            if cursor >= bytes.len() {
                return None;
            }
            let remaining_before = bytes.len() - cursor;
            let mut nonce_input: &[u8] = &bytes[cursor..];
            let nonce = u64::compact_decode_from(&mut nonce_input).ok()?;
            cursor += remaining_before - nonce_input.len();
            if cursor.checked_add(1)? > bytes.len() {
                return None;
            }
            let k_suffix_len = bytes[cursor] as usize;
            cursor += 1;
            if cursor.checked_add(k_suffix_len)? > bytes.len() {
                return None;
            }
            // Reconstruct full key = shared prefix ++ per-entry suffix.
            let full_key_len = prefix_len.checked_add(k_suffix_len)?;
            let mut full_key = Vec::with_capacity(full_key_len);
            full_key.extend_from_slice(prefix);
            full_key.extend_from_slice(&bytes[cursor..cursor + k_suffix_len]);
            cursor += k_suffix_len;
            let mut body: &[u8] = &full_key;
            let key = K::compact_decode_from(&mut body).ok()?;
            if !body.is_empty() {
                return None;
            }
            if cursor.checked_add(1)? > bytes.len() {
                return None;
            }
            let v_len = bytes[cursor] as usize;
            cursor += 1;
            if cursor.checked_add(v_len)? > bytes.len() {
                return None;
            }
            let mut body: &[u8] = &bytes[cursor..cursor + v_len];
            let value = V::compact_decode_from(&mut body).ok()?;
            cursor += v_len;
            entries.push(Entry { key, nonce, value });
        }

        let mut children: Vec<NodeId> = Vec::with_capacity(children_len);
        let mut child_counts: Vec<u64> = Vec::with_capacity(children_len);
        let mut child_entry_counts: Vec<u32> = Vec::with_capacity(children_len);
        for _ in 0..children_len {
            let remaining_before = bytes.len() - cursor;
            let mut input: &[u8] = &bytes[cursor..];
            let node_id = u64::compact_decode_from(&mut input).ok()?;
            cursor += remaining_before - input.len();

            let remaining_before = bytes.len() - cursor;
            let mut input: &[u8] = &bytes[cursor..];
            let subtree_count = u64::compact_decode_from(&mut input).ok()?;
            cursor += remaining_before - input.len();

            if cursor >= bytes.len() {
                return None;
            }
            let own_entry_count = bytes[cursor];
            cursor += 1;
            children.push(NodeId(node_id));
            child_counts.push(subtree_count);
            child_entry_counts.push(u32::from(own_entry_count));
        }

        if cursor != bytes.len() {
            return None;
        }

        Some(Self {
            entries,
            children,
            child_counts,
            child_entry_counts,
        })
    }
}

fn encode_codec_bytes<T: CompactCodec>(v: &T) -> Vec<u8> {
    let len = v.compact_encoded_len();
    let mut buf = alloc::vec![0u8; len];
    let mut cursor: &mut [u8] = &mut buf;
    v.compact_encode_to(&mut cursor);
    buf
}

fn derive_cell_key(namespace: &[u8], suffix: &[u8]) -> StorageKey {
    let mut preimage = Vec::with_capacity(namespace.len() + 1 + suffix.len());
    preimage.extend_from_slice(namespace);
    preimage.push(0);
    preimage.extend_from_slice(suffix);
    StorageKey(pvm_contract_types::keccak256(&preimage))
}

pub struct OrderedIndex<K, V, const T: usize = 2> {
    root_key: StorageKey,
    root_cell_key: StorageKey,
    next_id_cell_key: StorageKey,
    next_nonce_cell_key: StorageKey,
    _marker: PhantomData<(K, V)>,
}

impl<K, V, const T: usize> OrderedIndex<K, V, T> {
    pub fn new(namespace: &'static [u8], _host: Host) -> Self {
        assert!(T >= 2, "OrderedIndex: minimum degree T must be >= 2");
        // On-wire capacity limits: `own_entry_count` and `entries_len` are u8,
        // so a node holds at most 255 entries → 2T-1 ≤ 255 → T ≤ 128.
        assert!(
            T <= 128,
            "OrderedIndex: T > 128 exceeds u8 entries_len capacity (2T-1 > 255)"
        );
        let root_key = StorageKey(pvm_contract_types::keccak256(namespace));
        let root_cell_key = derive_cell_key(namespace, b"root");
        let next_id_cell_key = derive_cell_key(namespace, b"next_id");
        let next_nonce_cell_key = derive_cell_key(namespace, b"next_nonce");
        Self {
            root_key,
            root_cell_key,
            next_id_cell_key,
            next_nonce_cell_key,
            _marker: PhantomData,
        }
    }

    const fn max_keys() -> usize {
        2 * T - 1
    }

    fn cell_lazy(&self, host: &Host, key: StorageKey) -> Lazy<u64> {
        // SAFETY: `Lazy::new` is `unsafe` only because it bypasses the
        // `#[storage]` layout walker; its contract is that no two `Lazy`s
        // claim overlapping storage keys. The three cell keys
        // (root/next_id/next_nonce) are distinct `keccak256(namespace ++
        // suffix)` values, and node bodies live under `NodeId`-derived keys
        // in a disjoint subtree, so nothing overlaps. `offset` is 0 (full-slot u64).
        unsafe { Lazy::<u64>::new(key, 0, host.clone()) }
    }

    fn root_cell_lazy(&self, host: &Host) -> Lazy<u64> {
        self.cell_lazy(host, self.root_cell_key)
    }

    fn next_id_cell_lazy(&self, host: &Host) -> Lazy<u64> {
        self.cell_lazy(host, self.next_id_cell_key)
    }

    fn next_nonce_cell_lazy(&self, host: &Host) -> Lazy<u64> {
        self.cell_lazy(host, self.next_nonce_cell_key)
    }

    fn node_key(&self, host: &Host, id: NodeId) -> StorageKey {
        id.derive_slot(host, &self.root_key)
    }
}

impl<K, V, const T: usize> OrderedIndex<K, V, T>
where
    K: SolEncode + SolDecode + Ord + Clone + AsStorageKey + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
{
    fn load_node(&self, host: &Host, id: NodeId) -> Node<K, V> {
        let key = self.node_key(host, id);
        let bytes = storage_get_bytes(host, key.as_bytes()).unwrap_or_default();
        if bytes.is_empty() {
            panic!("OrderedIndexMissingNode");
        }
        match Node::decode(&bytes) {
            Some(n) => n,
            None => panic!("OrderedIndexCorruptNode"),
        }
    }

    fn store_node(&self, host: &Host, id: NodeId, node: &Node<K, V>) {
        self.assert_node_shape(node);
        self.assert_node_size(node);
        let key = self.node_key(host, id);
        let bytes = node.encode();
        storage_set_bytes(host, key.as_bytes(), &bytes);
    }

    fn free_node(&self, host: &Host, id: NodeId) {
        let key = self.node_key(host, id);
        storage_clear_value(host, key.as_bytes());
    }

    fn alloc_node(&self, host: &Host, node: &Node<K, V>) -> NodeId {
        let raw = self.next_id_cell_lazy(host).get();
        let id = if raw == 0 { 1 } else { raw };
        let next = id.checked_add(1).expect("OrderedIndexNodeIdOverflow");
        self.next_id_cell_lazy(host).set(&next);
        self.store_node(host, NodeId(id), node);
        NodeId(id)
    }

    fn alloc_nonce(&self, host: &Host) -> u64 {
        let n = self.next_nonce_cell_lazy(host).get();
        let next = n.checked_add(1).expect("OrderedIndexNonceOverflow");
        self.next_nonce_cell_lazy(host).set(&next);
        n
    }

    fn root_id(&self, host: &Host) -> Option<NodeId> {
        let v = self.root_cell_lazy(host).get();
        if v == 0 {
            None
        } else {
            Some(NodeId(v))
        }
    }

    fn set_root_id(&self, host: &Host, id: NodeId) {
        self.root_cell_lazy(host).set(&id.0);
    }

    fn clear_root_id(&self, host: &Host) {
        self.root_cell_lazy(host).set(&0);
    }

    fn refresh_child_mirrors(&self, host: &Host, node: &mut Node<K, V>, child_idx: usize) {
        let child = self.load_node(host, node.children[child_idx]);
        node.child_counts[child_idx] = child.subtree_count();
        node.child_entry_counts[child_idx] = child.entries.len() as u32;
    }

    fn assert_node_shape(&self, node: &Node<K, V>) {
        if node.entries.len() > Self::max_keys() {
            panic!("OrderedIndexNodeTooManyEntries");
        }
        if node.is_leaf() {
            if !node.child_counts.is_empty() || !node.child_entry_counts.is_empty() {
                panic!("OrderedIndexLeafHasMirrors");
            }
        } else {
            let expected_children = node.entries.len() + 1;
            if node.children.len() != expected_children
                || node.child_counts.len() != expected_children
                || node.child_entry_counts.len() != expected_children
            {
                panic!("OrderedIndexBadChildMirrors");
            }
        }
        for i in 1..node.entries.len() {
            let prev = &node.entries[i - 1];
            let curr = &node.entries[i];
            if prev.key > curr.key || (prev.key == curr.key && prev.nonce >= curr.nonce) {
                panic!("OrderedIndexUnsortedNode");
            }
        }
    }

    fn assert_node_size(&self, node: &Node<K, V>) {
        if node.encode().len() > MAX_STORAGE_VALUE_BYTES {
            panic!("OrderedIndexNodeTooLarge");
        }
    }

    pub fn len(&self, host: &Host) -> u64 {
        match self.root_id(host) {
            None => 0,
            Some(id) => self.load_node(host, id).subtree_count(),
        }
    }

    pub fn is_empty(&self, host: &Host) -> bool {
        self.len(host) == 0
    }

    pub fn insert(&self, host: &Host, key: &K, value: &V) -> u64 {
        let nonce = self.alloc_nonce(host);
        let entry = Entry {
            key: key.clone(),
            nonce,
            value: value.clone(),
        };
        match self.root_id(host) {
            None => {
                let mut root = Node::<K, V>::leaf();
                root.entries.push(entry);
                let id = self.alloc_node(host, &root);
                self.set_root_id(host, id);
            }
            Some(root_id) => {
                let root = self.load_node(host, root_id);
                if root.entries.len() == Self::max_keys() {
                    let mut new_root = Node {
                        entries: Vec::new(),
                        children: alloc::vec![root_id],
                        child_counts: alloc::vec![root.subtree_count()],
                        child_entry_counts: alloc::vec![root.entries.len() as u32],
                    };
                    self.split_child(host, &mut new_root, 0);
                    let new_root_id = self.alloc_node(host, &new_root);
                    self.set_root_id(host, new_root_id);
                    self.insert_nonfull(host, new_root_id, entry);
                } else {
                    self.insert_nonfull(host, root_id, entry);
                }
            }
        }
        nonce
    }

    pub fn get_first(&self, host: &Host, key: &K) -> Option<V> {
        let mut id = self.root_id(host)?;
        let mut candidate: Option<V> = None;
        loop {
            let node = self.load_node(host, id);
            let pos = node.lower_bound_key(key);
            if pos < node.entries.len() && node.entries[pos].key == *key {
                candidate = Some(node.entries[pos].value.clone());
            }
            if node.is_leaf() {
                return candidate;
            }
            id = node.children[pos];
        }
    }

    pub fn remove_by_nonce(&self, host: &Host, key: &K, nonce: u64) -> Option<V> {
        let root_id = self.root_id(host)?;
        let removed = self.remove_from(host, root_id, key, nonce);
        if removed.is_some() {
            let root = self.load_node(host, root_id);
            if root.entries.is_empty() {
                if root.is_leaf() {
                    self.free_node(host, root_id);
                    self.clear_root_id(host);
                } else {
                    let new_root = root.children[0];
                    self.free_node(host, root_id);
                    self.set_root_id(host, new_root);
                }
            }
        }
        removed
    }

    pub fn remove_first(&self, host: &Host, key: &K) -> Option<V> {
        let nonce = self.find_first_nonce(host, key)?;
        self.remove_by_nonce(host, key, nonce)
    }

    pub fn remove(&self, host: &Host, key: &K, value: &V) -> bool
    where
        V: PartialEq,
    {
        match self.find_nonce_for(host, key, value) {
            Some(n) => self.remove_by_nonce(host, key, n).is_some(),
            None => false,
        }
    }

    pub fn select(&self, host: &Host, mut rank: u64) -> Option<(K, V)> {
        let mut id = self.root_id(host)?;
        loop {
            let node = self.load_node(host, id);
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
                rank -= 1;
                i += 1;
            }
        }
    }

    pub fn rank_of_key(&self, host: &Host, key: &K) -> u64 {
        let mut id = match self.root_id(host) {
            Some(id) => id,
            None => return 0,
        };
        let mut rank: u64 = 0;
        loop {
            let node = self.load_node(host, id);
            let pos = node.lower_bound_key(key);
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

    pub fn range(
        &self,
        host: &Host,
        from: Bound<&K>,
        to: Bound<&K>,
        offset: u64,
        limit: u64,
    ) -> Vec<(K, V)> {
        let mut out: Vec<(K, V)> = Vec::new();
        if limit == 0 {
            return out;
        }
        let mut skipped: u64 = 0;
        if let Some(root) = self.root_id(host) {
            self.range_walk(host, root, from, to, offset, limit, &mut skipped, &mut out);
        }
        out
    }

    fn insert_nonfull(&self, host: &Host, node_id: NodeId, entry: Entry<K, V>) {
        let mut node = self.load_node(host, node_id);
        let pos = node.lower_bound_entry(&entry.key, entry.nonce);

        if node.is_leaf() {
            node.entries.insert(pos, entry);
            self.store_node(host, node_id, &node);
            return;
        }

        let mut child_idx = pos;
        let child = self.load_node(host, node.children[child_idx]);
        if child.entries.len() == Self::max_keys() {
            self.split_child(host, &mut node, child_idx);
            let sep = &node.entries[child_idx];
            let goes_right = (entry.key.cmp(&sep.key))
                .then(entry.nonce.cmp(&sep.nonce))
                == core::cmp::Ordering::Greater;
            if goes_right {
                child_idx += 1;
            }
        }

        let descend = node.children[child_idx];
        self.insert_nonfull(host, descend, entry);
        self.refresh_child_mirrors(host, &mut node, child_idx);
        self.store_node(host, node_id, &node);
    }

    fn split_child(&self, host: &Host, parent: &mut Node<K, V>, i: usize) {
        let left_id = parent.children[i];
        let mut left = self.load_node(host, left_id);

        let right_entries: Vec<Entry<K, V>> = left.entries.drain(T..).collect();
        let middle = match left.entries.pop() {
            Some(entry) => entry,
            None => panic!("OrderedIndexMissingMedian"),
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

        self.store_node(host, left_id, &left);
        let right_id = self.alloc_node(host, &right);

        parent.entries.insert(i, middle);
        parent.children.insert(i + 1, right_id);
        parent.child_counts[i] = left_count;
        parent.child_counts.insert(i + 1, right_count);
        parent.child_entry_counts[i] = left_entry_count;
        parent.child_entry_counts.insert(i + 1, right_entry_count);
    }

    fn remove_from(&self, host: &Host, node_id: NodeId, k: &K, nonce: u64) -> Option<V> {
        let mut node = self.load_node(host, node_id);
        let pos = node.lower_bound_entry(k, nonce);
        let at_pos = pos < node.entries.len()
            && node.entries[pos].key == *k
            && node.entries[pos].nonce == nonce;

        if at_pos {
            if node.is_leaf() {
                let removed = node.entries.remove(pos).value;
                self.store_node(host, node_id, &node);
                return Some(removed);
            }
            let original = node.entries[pos].value.clone();

            if node.child_entry_counts[pos] >= T as u32 {
                let pred = self.find_max(host, node.children[pos]);
                self.remove_from(host, node.children[pos], &pred.key, pred.nonce);
                node.entries[pos] = pred;
                self.refresh_child_mirrors(host, &mut node, pos);
                self.store_node(host, node_id, &node);
            } else if node.child_entry_counts[pos + 1] >= T as u32 {
                let succ = self.find_min(host, node.children[pos + 1]);
                self.remove_from(host, node.children[pos + 1], &succ.key, succ.nonce);
                node.entries[pos] = succ;
                self.refresh_child_mirrors(host, &mut node, pos + 1);
                self.store_node(host, node_id, &node);
            } else {
                self.merge_children(host, &mut node, pos);
                let merged_id = node.children[pos];
                self.store_node(host, node_id, &node);
                let _ = self.remove_from(host, merged_id, k, nonce);
                self.refresh_child_mirrors(host, &mut node, pos);
                self.store_node(host, node_id, &node);
            }
            return Some(original);
        }

        if node.is_leaf() {
            return None;
        }
        let descend = self.descend_prepared(host, &mut node, pos);
        let child_id = node.children[descend];
        self.store_node(host, node_id, &node);
        let result = self.remove_from(host, child_id, k, nonce);
        if result.is_some() {
            self.refresh_child_mirrors(host, &mut node, descend);
            self.store_node(host, node_id, &node);
        }
        result
    }

    fn descend_prepared(&self, host: &Host, node: &mut Node<K, V>, pos: usize) -> usize {
        if node.child_entry_counts[pos] >= T as u32 {
            return pos;
        }
        let has_left = pos > 0;
        let has_right = pos + 1 < node.children.len();

        if has_left && node.child_entry_counts[pos - 1] >= T as u32 {
            self.borrow_from_left(host, node, pos);
            pos
        } else if has_right && node.child_entry_counts[pos + 1] >= T as u32 {
            self.borrow_from_right(host, node, pos);
            pos
        } else if has_right {
            self.merge_children(host, node, pos);
            pos
        } else {
            self.merge_children(host, node, pos - 1);
            pos - 1
        }
    }

    fn borrow_from_left(&self, host: &Host, node: &mut Node<K, V>, pos: usize) {
        let left_id = node.children[pos - 1];
        let child_id = node.children[pos];
        let mut left = self.load_node(host, left_id);
        let mut child = self.load_node(host, child_id);

        let separator = node.entries[pos - 1].clone();
        let new_separator = match left.entries.pop() {
            Some(entry) => entry,
            None => panic!("OrderedIndexBorrowLeftEmpty"),
        };
        child.entries.insert(0, separator);
        node.entries[pos - 1] = new_separator;

        if !left.is_leaf() {
            let moved_child = match left.children.pop() {
                Some(value) => value,
                None => panic!("OrderedIndexMissingLeftChild"),
            };
            let moved_count = match left.child_counts.pop() {
                Some(value) => value,
                None => panic!("OrderedIndexMissingLeftCount"),
            };
            let moved_entry_count = match left.child_entry_counts.pop() {
                Some(value) => value,
                None => panic!("OrderedIndexMissingLeftEntryCount"),
            };
            child.children.insert(0, moved_child);
            child.child_counts.insert(0, moved_count);
            child.child_entry_counts.insert(0, moved_entry_count);
        }

        node.child_counts[pos - 1] = left.subtree_count();
        node.child_counts[pos] = child.subtree_count();
        node.child_entry_counts[pos - 1] = left.entries.len() as u32;
        node.child_entry_counts[pos] = child.entries.len() as u32;

        self.store_node(host, left_id, &left);
        self.store_node(host, child_id, &child);
    }

    fn borrow_from_right(&self, host: &Host, node: &mut Node<K, V>, pos: usize) {
        let child_id = node.children[pos];
        let right_id = node.children[pos + 1];
        let mut child = self.load_node(host, child_id);
        let mut right = self.load_node(host, right_id);

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

        self.store_node(host, child_id, &child);
        self.store_node(host, right_id, &right);
    }

    fn merge_children(&self, host: &Host, node: &mut Node<K, V>, pos: usize) {
        let left_id = node.children[pos];
        let right_id = node.children[pos + 1];
        let mut left = self.load_node(host, left_id);
        let right = self.load_node(host, right_id);

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

        self.store_node(host, left_id, &left);
        self.free_node(host, right_id);
    }

    fn find_max(&self, host: &Host, mut id: NodeId) -> Entry<K, V> {
        loop {
            let node = self.load_node(host, id);
            if node.is_leaf() {
                return match node.entries.last() {
                    Some(entry) => entry.clone(),
                    None => panic!("OrderedIndexEmptyMaxLeaf"),
                };
            }
            id = match node.children.last() {
                Some(child_id) => *child_id,
                None => panic!("OrderedIndexMissingMaxChild"),
            };
        }
    }

    fn find_min(&self, host: &Host, mut id: NodeId) -> Entry<K, V> {
        loop {
            let node = self.load_node(host, id);
            if node.is_leaf() {
                return match node.entries.first() {
                    Some(entry) => entry.clone(),
                    None => panic!("OrderedIndexEmptyMinLeaf"),
                };
            }
            id = node.children[0];
        }
    }

    fn find_first_nonce(&self, host: &Host, k: &K) -> Option<u64> {
        let mut id = self.root_id(host)?;
        let mut candidate: Option<u64> = None;
        loop {
            let node = self.load_node(host, id);
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

    fn find_nonce_for(&self, host: &Host, k: &K, v: &V) -> Option<u64>
    where
        V: PartialEq,
    {
        let rank = self.rank_of_key(host, k);
        let total = self.len(host);
        let mut cursor = rank;
        while cursor < total {
            let (ck, nonce, cv) = self.select_with_nonce(host, cursor)?;
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

    fn select_with_nonce(&self, host: &Host, mut rank: u64) -> Option<(K, u64, V)> {
        let mut id = self.root_id(host)?;
        loop {
            let node = self.load_node(host, id);
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

    #[allow(clippy::too_many_arguments)]
    fn range_walk(
        &self,
        host: &Host,
        node_id: NodeId,
        from: Bound<&K>,
        to: Bound<&K>,
        offset: u64,
        limit: u64,
        skipped: &mut u64,
        out: &mut Vec<(K, V)>,
    ) -> bool {
        if out.len() as u64 == limit {
            return true;
        }
        let node = self.load_node(host, node_id);

        let start = match from {
            Bound::Unbounded => 0,
            Bound::Included(k) => node.lower_bound_key(k),
            Bound::Excluded(k) => node.upper_bound_key(k),
        };

        for i in start..=node.entries.len() {
            // Subtree-overlap prune: skip children whose key interval does
            // not overlap [from, to). The entry-emission check below must
            // still run even when the child is skipped — internal nodes
            // hold real entries in this B-tree, not separator keys.
            let descend = !node.is_leaf() && node.child_interval_overlaps(i, from, to);
            if descend
                && self.range_walk(
                    host,
                    node.children[i],
                    from,
                    to,
                    offset,
                    limit,
                    skipped,
                    out,
                )
            {
                return true;
            }
            if out.len() as u64 == limit {
                return true;
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

    #[cfg(test)]
    fn walk_all_nodes(&self, host: &Host, visit: &mut dyn FnMut(&Node<K, V>)) {
        if let Some(root) = self.root_id(host) {
            self.walk_node(host, root, visit);
        }
    }

    #[cfg(test)]
    fn walk_node(&self, host: &Host, id: NodeId, visit: &mut dyn FnMut(&Node<K, V>)) {
        let node = self.load_node(host, id);
        visit(&node);
        for child in &node.children {
            self.walk_node(host, *child, visit);
        }
    }
}

impl<
        K: SolEncode + SolDecode + Clone + CompactCodec + Ord,
        V: SolEncode + SolDecode + Clone + CompactCodec,
    > Node<K, V>
{
    fn lower_bound_key(&self, k: &K) -> usize {
        self.entries.partition_point(|e| e.key < *k)
    }

    fn upper_bound_key(&self, k: &K) -> usize {
        self.entries.partition_point(|e| e.key <= *k)
    }

    fn lower_bound_entry(&self, k: &K, nonce: u64) -> usize {
        self.entries
            .partition_point(|e| e.key < *k || (e.key == *k && e.nonce < nonce))
    }

    /// Returns true iff the key interval of `children[child_idx]` — the open
    /// interval `(lo, hi)` with sentinels at the ends — may contain at least
    /// one key in `[from, to)`. Conservative: may return `true` for a child
    /// that ultimately contributes no in-range entries (e.g., empty range
    /// spanning `k`); the deeper recursion then finds nothing — wasted read,
    /// not a correctness issue. Strict inequalities handle Included/Excluded
    /// uniformly: a child with `hi = k` holds no keys for `from = Excluded(k)`
    /// (want `> k`); a child with `lo = k` holds no keys for
    /// `to = Excluded(k)` (want `< k`).
    fn child_interval_overlaps(
        &self,
        child_idx: usize,
        from: Bound<&K>,
        to: Bound<&K>,
    ) -> bool {
        // Child C_i holds (key, nonce) tuples strictly between
        // (entries[i-1].key, entries[i-1].nonce) and (entries[i].key, entries[i].nonce).
        // Duplicate keys straddle node boundaries: when entries[i].key == k, C_i can
        // still hold entries with key == k (and a smaller nonce), and C_{i+1} can hold
        // entries with key == k (and a larger nonce). An Included(k) bound must
        // therefore use >= / <= so those duplicate-bearing children are not pruned;
        // Excluded(k) stays strict because == k is out of range by definition.
        let from_ok = match from {
            Bound::Unbounded => true,
            Bound::Included(k) => child_idx >= self.entries.len() || self.entries[child_idx].key >= *k,
            Bound::Excluded(k) => child_idx >= self.entries.len() || self.entries[child_idx].key > *k,
        };
        let to_ok = match to {
            Bound::Unbounded => true,
            Bound::Included(k) => child_idx == 0 || self.entries[child_idx - 1].key <= *k,
            Bound::Excluded(k) => child_idx == 0 || self.entries[child_idx - 1].key < *k,
        };
        from_ok && to_ok
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::collections::BTreeMap;
    use alloc::rc::Rc;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::ops::Bound;
    use pvm_contract_types::{Host, MockHostBuilder};
    use proptest::prelude::*;

    use super::{Node, NodeId, OrderedIndex, MAX_STORAGE_VALUE_BYTES};

    fn host() -> Host {
        Host::from_dyn(Rc::new(MockHostBuilder::new().build()))
    }

    fn index(host: &Host) -> OrderedIndex<String, u64, 2> {
        OrderedIndex::new(b"test-ns", host.clone())
    }

    fn short_key() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[a-z]{1,8}").unwrap()
    }

    fn smallest_nonce_for(
        oracle: &BTreeMap<(String, u64), u64>,
        key: &String,
    ) -> Option<(u64, u64)> {
        oracle
            .iter()
            .filter(|((k, _), _)| k == key)
            .min_by_key(|((_, n), _)| *n)
            .map(|((_, n), v)| (*n, *v))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn insert_get_first_roundtrip(
            ops in proptest::collection::vec((short_key(), any::<u64>()), 0..80)
        ) {
            let host = host();
            let idx = index(&host);
            let mut oracle: BTreeMap<(String, u64), u64> = BTreeMap::new();
            for (k, v) in &ops {
                let n = idx.insert(&host, k, v);
                oracle.insert((k.clone(), n), *v);
            }
            let mut distinct_keys: Vec<String> = ops.iter().map(|(k, _)| k.clone()).collect();
            distinct_keys.sort();
            distinct_keys.dedup();
            for k in &distinct_keys {
                let min_entry = smallest_nonce_for(&oracle, k).map(|(_, v)| v);
                prop_assert_eq!(idx.get_first(&host, k), min_entry);
            }
            prop_assert_eq!(idx.len(&host) as usize, oracle.len());
        }

        #[test]
        fn range_for_single_key(
            ops in proptest::collection::vec((short_key(), any::<u64>()), 0..80)
        ) {
            let host = host();
            let idx = index(&host);
            let mut oracle: BTreeMap<(String, u64), u64> = BTreeMap::new();
            for (k, v) in &ops {
                let n = idx.insert(&host, k, v);
                oracle.insert((k.clone(), n), *v);
            }
            let mut distinct_keys: Vec<String> = ops.iter().map(|(k, _)| k.clone()).collect();
            distinct_keys.sort();
            distinct_keys.dedup();
            for k in &distinct_keys {
                let actual = idx.range(
                    &host,
                    Bound::Included(k),
                    Bound::Included(k),
                    0,
                    u64::MAX,
                );
                let mut expected_pairs: Vec<(String, u64)> = oracle
                    .iter()
                    .filter(|((kk, _), _)| kk == k)
                    .map(|((kk, _), v)| (kk.clone(), *v))
                    .collect();
                expected_pairs.sort_by_key(|p| {
                    oracle
                        .iter()
                        .find(|((kk, _), vv)| kk == &p.0 && **vv == p.1)
                        .map(|((_, n), _)| *n)
                        .unwrap_or(0)
                });
                prop_assert_eq!(actual, expected_pairs);
            }
        }

        #[test]
        fn range_between_bounds(
            ops in proptest::collection::vec((short_key(), any::<u64>()), 0..80)
        ) {
            let host = host();
            let idx = index(&host);
            let mut oracle: BTreeMap<(String, u64), u64> = BTreeMap::new();
            for (k, v) in &ops {
                let n = idx.insert(&host, k, v);
                oracle.insert((k.clone(), n), *v);
            }
            let mut distinct_keys: Vec<String> = ops.iter().map(|(k, _)| k.clone()).collect();
            distinct_keys.sort();
            distinct_keys.dedup();
            if distinct_keys.len() < 2 {
                return Ok(());
            }
            let lo = distinct_keys[0].clone();
            let hi = distinct_keys[distinct_keys.len() - 1].clone();
            let actual = idx.range(
                &host,
                Bound::Included(&lo),
                Bound::Included(&hi),
                0,
                u64::MAX,
            );
            let mut expected: Vec<(String, u64)> = oracle
                .iter()
                .filter(|((k, _), _)| k.as_str() >= lo.as_str() && k.as_str() <= hi.as_str())
                .map(|((k, _), v)| (k.clone(), *v))
                .collect();
            expected.sort_by(|a, b| {
                let na = smallest_nonce_for(&oracle, &a.0).map(|(n, _)| n).unwrap_or(0);
                let nb = smallest_nonce_for(&oracle, &b.0).map(|(n, _)| n).unwrap_or(0);
                a.0.cmp(&b.0).then(na.cmp(&nb))
            });
            prop_assert_eq!(actual, expected);
        }

        #[test]
        fn btree_invariants(
            ops in proptest::collection::vec((short_key(), any::<u64>()), 0..100)
        ) {
            let host = host();
            let idx = index(&host);
            for (k, v) in &ops {
                idx.insert(&host, k, v);
            }
            let max_keys = 2 * 2 - 1;
            idx.walk_all_nodes(&host, &mut |node: &Node<String, u64>| {
                assert!(node.entries.len() <= max_keys, "node too many entries");
                if !node.children.is_empty() {
                    assert_eq!(node.children.len(), node.entries.len() + 1);
                    assert_eq!(node.child_counts.len(), node.entries.len() + 1);
                    assert_eq!(node.child_entry_counts.len(), node.entries.len() + 1);
                }
            });
            if let Some(root_id) = idx.root_id(&host) {
                let mut depths: Vec<usize> = Vec::new();
                walk_depths(&idx, &host, root_id, 0, &mut depths);
                if !depths.is_empty() {
                    let first = depths[0];
                    for d in &depths {
                        assert_eq!(*d, first, "leaves at unequal depths");
                    }
                }
            }
            idx.walk_all_nodes(&host, &mut |node: &Node<String, u64>| {
                for i in 0..node.children.len() {
                    let child = idx.load_node(&host, node.children[i]);
                    assert_eq!(node.child_counts[i], child.subtree_count());
                    assert_eq!(node.child_entry_counts[i], child.entries.len() as u32);
                }
            });
        }

        #[test]
        fn size_cap(
            ops in proptest::collection::vec((short_key(), any::<u64>()), 0..80)
        ) {
            let host = host();
            let idx = index(&host);
            for (k, v) in &ops {
                idx.insert(&host, k, v);
            }
            idx.walk_all_nodes(&host, &mut |node: &Node<String, u64>| {
                let size = node.encode().len();
                assert!(size <= MAX_STORAGE_VALUE_BYTES, "node too large: {}", size);
            });
        }

        #[test]
        fn remove_first_idempotence(
            ops in proptest::collection::vec((short_key(), any::<u64>()), 1..80)
        ) {
            let host = host();
            let idx = index(&host);
            let mut oracle: BTreeMap<(String, u64), u64> = BTreeMap::new();
            for (k, v) in &ops {
                let n = idx.insert(&host, k, v);
                oracle.insert((k.clone(), n), *v);
            }
            let pre_len = idx.len(&host);
            let mut distinct_keys: Vec<String> = ops.iter().map(|(k, _)| k.clone()).collect();
            distinct_keys.sort();
            distinct_keys.dedup();
            for k in &distinct_keys {
                let removed = idx.remove_first(&host, k);
                prop_assert!(removed.is_some());
                if let Some(min_nonce) = oracle
                    .keys()
                    .filter(|(kk, _)| kk == k)
                    .map(|(_, n)| *n)
                    .min()
                {
                    oracle.remove(&(k.clone(), min_nonce));
                }
                let new_first = idx.get_first(&host, k);
                let expected_new = oracle
                    .iter()
                    .filter(|((kk, _), _)| kk == k)
                    .map(|((_, n), v)| (*n, *v))
                    .min_by_key(|(n, _)| *n);
                match expected_new {
                    Some((_, v)) => prop_assert_eq!(new_first, Some(v)),
                    None => prop_assert_eq!(new_first, None),
                }
            }
            prop_assert_eq!(idx.len(&host) as usize, oracle.len());
            prop_assert_eq!(pre_len as usize, oracle.len() + distinct_keys.len());
        }

        #[test]
        fn pagination(
            ops in proptest::collection::vec((short_key(), any::<u64>()), 4..80)
        ) {
            let host = host();
            let idx = index(&host);
            for (k, v) in &ops {
                idx.insert(&host, k, v);
            }
            let total = idx.range(
                &host,
                Bound::Unbounded,
                Bound::Unbounded,
                0,
                u64::MAX,
            );
            let window: u64 = 7;
            let mut page_start: u64 = 0;
            let mut collected: Vec<(String, u64)> = Vec::new();
            loop {
                let page = idx.range(
                    &host,
                    Bound::Unbounded,
                    Bound::Unbounded,
                    page_start,
                    window,
                );
                if page.is_empty() {
                    break;
                }
                collected.extend(page.iter().cloned());
                page_start += page.len() as u64;
                if (page.len() as u64) < window {
                    break;
                }
            }
            prop_assert_eq!(collected, total);
        }
    }

    fn walk_depths(
        idx: &OrderedIndex<String, u64, 2>,
        host: &Host,
        id: NodeId,
        depth: usize,
        depths: &mut Vec<usize>,
    ) {
        let node = idx.load_node(host, id);
        if node.is_leaf() {
            depths.push(depth);
        } else {
            for c in &node.children {
                walk_depths(idx, host, *c, depth + 1, depths);
            }
        }
    }
}
