extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;
use core::ops::Bound;

use parity_scale_codec::{Compact, Decode, Encode};
use pvm_contract_types::{DecodeError, Host, SolDecode, SolEncode};

use crate::{
    AsStorageKey, Lazy, MAX_STORAGE_VALUE_BYTES, StorageKey, storage_clear_value,
    storage_get_bytes, storage_set_bytes,
};

/// Compact raw-byte body encoding for the OrderedIndex B+-tree node's
/// per-entry K and V. Replaces the 32-byte-aligned `SolEncode` ABI body
/// (a `String` of N UTF-8 bytes is N here vs 32+N+pad there), which lifts
/// the achievable fanout far above what the 416-byte per-slot cap would
/// otherwise allow and roughly halves slot-reads per range query.
///
/// `compact_encoded_len` returns the **body** byte count, not the on-wire
/// length: the OrderedIndex node writes its own 1-byte length headers in
/// front of the body. Callers that need a length prefix in a different
/// framing must add it themselves.
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
        Compact(*self).encoded_size()
    }

    fn compact_encode_to<'a>(&self, out: &'a mut &'a mut [u8]) {
        let encoded = Compact(*self).encode();
        let n = encoded.len();
        out[..n].copy_from_slice(&encoded);
        let (_, rest) = out.split_at_mut(n);
        *out = rest;
    }

    fn compact_decode_from(input: &mut &[u8]) -> Result<Self, DecodeError> {
        let Compact(value) = Compact::<u64>::decode(input).map_err(|_| DecodeError)?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(pub u64);

impl AsStorageKey for NodeId {
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
        self.0.derive_slot(host, root)
    }
}

/// Branded nonce — the per-insert monotonic disambiguator that orders
/// duplicate keys. A newtype, not a bare `u64`, so a nonce can never be
/// transposed with a value, an id, or a subtree count in a domain position
/// (no primitive obsession).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Nonce(pub u64);

/// Branded subtree entry-count mirror carried in a parent's `ChildRef`. The
/// total number of leaf entries reachable through that child's subtree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SubtreeCount(pub u64);

/// Branded own-entry-count mirror carried in a parent's `ChildRef`. The
/// number of entries (leaf) or separators (internal) the child node holds
/// directly. Used by the merge/borrow min-degree checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryCount(pub u32);

/// The ONLY place data lives in a B+ tree: a (key, nonce, value) tuple in a
/// leaf node.
struct LeafEntry<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
> {
    key: K,
    nonce: Nonce,
    value: V,
}

impl<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
> Clone for LeafEntry<K, V>
{
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            nonce: self.nonce,
            value: self.value.clone(),
        }
    }
}

/// A separator in an internal node: a (key, nonce) routing tuple with NO
/// value. Separators direct descent; they never carry payload (B+ semantics).
struct Separator<K: SolEncode + SolDecode + Clone + CompactCodec> {
    key: K,
    nonce: Nonce,
}

impl<K: SolEncode + SolDecode + Clone + CompactCodec> Clone for Separator<K> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            nonce: self.nonce,
        }
    }
}

/// A parent's per-child mirror: ONE struct, never parallel vectors that can
/// desync. `id` locates the child node; `subtree_count` and `entry_count`
/// mirror the child's totals for O(1) rank accounting and min-degree checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChildRef {
    id: NodeId,
    subtree_count: SubtreeCount,
    entry_count: EntryCount,
}

/// Typed decode failures — one variant per distinct failure mode, so a
/// caller can branch on the real cause instead of collapsing every failure
/// into a single `None`. Storage corruption is surfaced as a documented
/// defect at the shell, not silently swallowed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeDecodeError {
    /// The buffer ended before a field that the header promised.
    Truncated,
    /// An internal node declared a child count inconsistent with its
    /// separator count (must be `separators + 1`), or a count exceeded the
    /// representable maximum.
    BadChildCount,
    /// Bytes remained after the node was fully decoded.
    TrailingBytes,
    /// A K or V body failed its `CompactCodec` decode (bad UTF-8, LEB
    /// overflow, leftover bytes).
    BadCodec,
    /// A leaf's `next_leaf_id` trailer was malformed.
    BadNextLeaf,
}

/// Leaf-linked B+-tree node, split into two structurally-distinct variants
/// that carry ONLY their valid fields (illegal states unrepresentable):
///
///   * `Leaf` holds the (key, nonce, value) entries — the only place data
///     lives — plus a forward `next` link for straight-line range scans.
///   * `Internal` holds `separators` (routing keys, no value) and one more
///     `children` mirror than separators.
///
/// On-wire layout (BE throughout; flags bit 0 set = leaf):
///   header     : 1B flags | 1B count | 1B prefix_len
///       (children_len dropped: leaf has 0, internal has count+1 — derivable)
///   prefix     : [prefix_len bytes] — common prefix shared by ALL keys here
///   Leaf       : per entry: nonce(LEB) | 1B k_suffix_len | k_suffix | 1B v_len | v_body
///                trailer  : next_leaf_id(LEB, 0 = none)
///   Internal   : per separator: nonce(LEB) | 1B k_suffix_len | k_suffix
///                per child   : node_id(LEB) | subtree_count(LEB) | 1B own_entry_count
///
/// Separator-only internal slots cost ~6B vs the old fat ~16-23B (which
/// stored nonce+value bodies that range-descent never emits), so internal
/// fanout rises sharply at the same 416-byte cap. The node key derivation
/// (`mapping(uint256 => bytes)`) and the root/next_id/next_nonce cell keys
/// (`mapping(string => bytes)`) are byte-for-byte UNCHANGED; only the value
/// body under the node keys changes shape.
enum Node<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
> {
    Leaf {
        entries: Vec<LeafEntry<K, V>>,
        next: Option<NodeId>,
    },
    Internal {
        separators: Vec<Separator<K>>,
        children: Vec<ChildRef>,
    },
}

impl<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
> Clone for Node<K, V>
{
    fn clone(&self) -> Self {
        match self {
            Node::Leaf { entries, next } => Node::Leaf {
                entries: entries.clone(),
                next: *next,
            },
            Node::Internal {
                separators,
                children,
            } => Node::Internal {
                separators: separators.clone(),
                children: children.clone(),
            },
        }
    }
}

const FLAG_LEAF: u8 = 0x01;
const HEADER_LEN: usize = 1 + 1 + 1; // flags + count + prefix_len

impl<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
> Node<K, V>
{
    fn leaf() -> Self {
        Node::Leaf {
            entries: Vec::new(),
            next: None,
        }
    }

    fn is_leaf(&self) -> bool {
        matches!(self, Node::Leaf { .. })
    }

    fn children(&self) -> &[ChildRef] {
        match self {
            Node::Leaf { .. } => &[],
            Node::Internal { children, .. } => children,
        }
    }

    /// Number of routing slots: leaf entries or internal separators. This is
    /// the count written to the header.
    fn slot_count(&self) -> usize {
        match self {
            Node::Leaf { entries, .. } => entries.len(),
            Node::Internal { separators, .. } => separators.len(),
        }
    }

    /// Total leaf entries reachable through this node. Only leaves contribute
    /// entries directly; an internal node sums its child mirrors.
    fn subtree_count(&self) -> u64 {
        match self {
            Node::Leaf { entries, .. } => entries.len() as u64,
            Node::Internal { children, .. } => {
                children.iter().map(|c| c.subtree_count.0).sum::<u64>()
            }
        }
    }

    /// The first routing key in this node (smallest entry/separator key),
    /// used for prefix-compression and parent-separator computation.
    fn first_key(&self) -> Option<&K> {
        match self {
            Node::Leaf { entries, .. } => entries.first().map(|e| &e.key),
            Node::Internal { separators, .. } => separators.first().map(|s| &s.key),
        }
    }

    fn last_key(&self) -> Option<&K> {
        match self {
            Node::Leaf { entries, .. } => entries.last().map(|e| &e.key),
            Node::Internal { separators, .. } => separators.last().map(|s| &s.key),
        }
    }

    /// The common byte prefix of all routing keys in this node. For sorted
    /// keys this equals the common prefix of the first and last key (O(1)).
    fn key_prefix(&self) -> Vec<u8> {
        match (self.first_key(), self.last_key()) {
            (Some(first), Some(last)) if self.slot_count() >= 2 => {
                let fb = encode_codec_bytes(first);
                let lb = encode_codec_bytes(last);
                let plen = fb
                    .iter()
                    .zip(lb.iter())
                    .take_while(|(a, b)| a == b)
                    .count()
                    .min(255);
                fb[..plen].to_vec()
            }
            _ => Vec::new(),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let prefix = self.key_prefix();
        let prefix_len = prefix.len();
        let count = self.slot_count();
        let count_field = u8::try_from(count).expect("Node::encode: count > 255 (T>128)");

        let mut out = Vec::new();
        let flags: u8 = if self.is_leaf() { FLAG_LEAF } else { 0x00 };
        out.push(flags);
        out.push(count_field);
        out.push(u8::try_from(prefix_len).expect("prefix_len <= 255 (capped above)"));
        out.extend_from_slice(&prefix);

        match self {
            Node::Leaf { entries, next } => {
                for e in entries {
                    write_codec(&mut out, e.nonce.0);
                    write_key_suffix(&mut out, &e.key, prefix_len);
                    let v_len = u8::try_from(e.value.compact_encoded_len())
                        .expect("Node::encode: value body > 255 bytes");
                    out.push(v_len);
                    write_codec_body(&mut out, &e.value);
                }
                let trailer = next.map_or(0u64, |id| id.0);
                write_codec(&mut out, trailer);
            }
            Node::Internal {
                separators,
                children,
            } => {
                for s in separators {
                    write_codec(&mut out, s.nonce.0);
                    write_key_suffix(&mut out, &s.key, prefix_len);
                }
                for c in children {
                    write_codec(&mut out, c.id.0);
                    write_codec(&mut out, c.subtree_count.0);
                    out.push(
                        u8::try_from(c.entry_count.0)
                            .expect("Node::encode: own_entry_count > 255 (T>128)"),
                    );
                }
            }
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, NodeDecodeError> {
        if bytes.len() < HEADER_LEN {
            return Err(NodeDecodeError::Truncated);
        }
        let flags = bytes[0];
        let is_leaf = (flags & FLAG_LEAF) != 0;
        let count = bytes[1] as usize;
        let prefix_len = bytes[2] as usize;

        let mut cursor = HEADER_LEN;
        if cursor
            .checked_add(prefix_len)
            .ok_or(NodeDecodeError::Truncated)?
            > bytes.len()
        {
            return Err(NodeDecodeError::Truncated);
        }
        let prefix = bytes[cursor..cursor + prefix_len].to_vec();
        cursor += prefix_len;

        if is_leaf {
            let mut entries: Vec<LeafEntry<K, V>> = Vec::with_capacity(count);
            for _ in 0..count {
                let nonce = read_codec_u64(bytes, &mut cursor)?;
                let key = read_key(bytes, &mut cursor, &prefix)?;
                let v_len = read_byte(bytes, &mut cursor)? as usize;
                let value = read_codec_body::<V>(bytes, &mut cursor, v_len)?;
                entries.push(LeafEntry {
                    key,
                    nonce: Nonce(nonce),
                    value,
                });
            }
            let trailer =
                read_codec_u64(bytes, &mut cursor).map_err(|_| NodeDecodeError::BadNextLeaf)?;
            let next = if trailer == 0 {
                None
            } else {
                Some(NodeId(trailer))
            };
            if cursor != bytes.len() {
                return Err(NodeDecodeError::TrailingBytes);
            }
            Ok(Node::Leaf { entries, next })
        } else {
            let mut separators: Vec<Separator<K>> = Vec::with_capacity(count);
            for _ in 0..count {
                let nonce = read_codec_u64(bytes, &mut cursor)?;
                let key = read_key(bytes, &mut cursor, &prefix)?;
                separators.push(Separator {
                    key,
                    nonce: Nonce(nonce),
                });
            }
            let children_len = count.checked_add(1).ok_or(NodeDecodeError::BadChildCount)?;
            let mut children: Vec<ChildRef> = Vec::with_capacity(children_len);
            for _ in 0..children_len {
                let node_id = read_codec_u64(bytes, &mut cursor)?;
                let subtree_count = read_codec_u64(bytes, &mut cursor)?;
                let own_entry_count = read_byte(bytes, &mut cursor)?;
                children.push(ChildRef {
                    id: NodeId(node_id),
                    subtree_count: SubtreeCount(subtree_count),
                    entry_count: EntryCount(u32::from(own_entry_count)),
                });
            }
            if cursor != bytes.len() {
                return Err(NodeDecodeError::TrailingBytes);
            }
            Ok(Node::Internal {
                separators,
                children,
            })
        }
    }
}

fn write_codec(out: &mut Vec<u8>, v: u64) {
    let len = v.compact_encoded_len();
    let start = out.len();
    out.resize(start + len, 0);
    let mut cursor: &mut [u8] = &mut out[start..start + len];
    v.compact_encode_to(&mut cursor);
}

fn write_codec_body<T: CompactCodec>(out: &mut Vec<u8>, v: &T) {
    let len = v.compact_encoded_len();
    let start = out.len();
    out.resize(start + len, 0);
    let mut cursor: &mut [u8] = &mut out[start..start + len];
    v.compact_encode_to(&mut cursor);
}

fn write_key_suffix<K: CompactCodec>(out: &mut Vec<u8>, key: &K, prefix_len: usize) {
    let key_bytes = encode_codec_bytes(key);
    let suffix = &key_bytes[prefix_len..];
    let k_suffix_len = u8::try_from(suffix.len()).expect("Node::encode: key suffix > 255 bytes");
    out.push(k_suffix_len);
    out.extend_from_slice(suffix);
}

fn read_byte(bytes: &[u8], cursor: &mut usize) -> Result<u8, NodeDecodeError> {
    if *cursor >= bytes.len() {
        return Err(NodeDecodeError::Truncated);
    }
    let b = bytes[*cursor];
    *cursor += 1;
    Ok(b)
}

fn read_codec_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, NodeDecodeError> {
    if *cursor > bytes.len() {
        return Err(NodeDecodeError::Truncated);
    }
    let remaining_before = bytes.len() - *cursor;
    let mut input: &[u8] = &bytes[*cursor..];
    let v = u64::compact_decode_from(&mut input).map_err(|_| NodeDecodeError::BadCodec)?;
    *cursor += remaining_before - input.len();
    Ok(v)
}

fn read_key<K: CompactCodec>(
    bytes: &[u8],
    cursor: &mut usize,
    prefix: &[u8],
) -> Result<K, NodeDecodeError> {
    let k_suffix_len = read_byte(bytes, cursor)? as usize;
    if cursor
        .checked_add(k_suffix_len)
        .ok_or(NodeDecodeError::Truncated)?
        > bytes.len()
    {
        return Err(NodeDecodeError::Truncated);
    }
    let full_key_len = prefix
        .len()
        .checked_add(k_suffix_len)
        .ok_or(NodeDecodeError::Truncated)?;
    let mut full_key = Vec::with_capacity(full_key_len);
    full_key.extend_from_slice(prefix);
    full_key.extend_from_slice(&bytes[*cursor..*cursor + k_suffix_len]);
    *cursor += k_suffix_len;
    let mut body: &[u8] = &full_key;
    let key = K::compact_decode_from(&mut body).map_err(|_| NodeDecodeError::BadCodec)?;
    if !body.is_empty() {
        return Err(NodeDecodeError::BadCodec);
    }
    Ok(key)
}

fn read_codec_body<T: CompactCodec>(
    bytes: &[u8],
    cursor: &mut usize,
    body_len: usize,
) -> Result<T, NodeDecodeError> {
    if cursor
        .checked_add(body_len)
        .ok_or(NodeDecodeError::Truncated)?
        > bytes.len()
    {
        return Err(NodeDecodeError::Truncated);
    }
    let mut body: &[u8] = &bytes[*cursor..*cursor + body_len];
    let value = T::compact_decode_from(&mut body).map_err(|_| NodeDecodeError::BadCodec)?;
    *cursor += body_len;
    Ok(value)
}

fn encode_codec_bytes<T: CompactCodec>(v: &T) -> Vec<u8> {
    let len = v.compact_encoded_len();
    let mut buf = alloc::vec![0u8; len];
    let mut cursor: &mut [u8] = &mut buf;
    v.compact_encode_to(&mut cursor);
    buf
}

fn derive_cell_key(root: &StorageKey, suffix: &[u8]) -> StorageKey {
    // Solidity `mapping(string => bytes)` at slot `root`:
    //   keccak256(suffix ++ pad32(root))
    // (root is already a 32-byte slot, so pad32 is identity here.) Matches the
    // cast-validated `storage_derive_key_unpadded` helper output — the
    // `ordered_index_cell_and_node_keys_are_solidity_layout` test cross-checks
    // the two byte-for-byte, and `mapping_string_key_solidity_parity` proves
    // that helper against `cast index string`. This keeps OrderedIndex
    // Solidity-readable: a Solidity contract, `cast storage`, or any EVM tool
    // locates each cell exactly as it would a `mapping(string => bytes)` entry,
    // and matches the NodeId-derived node keys (`keccak256(pad32(id) ++ root)`,
    // i.e. `mapping(uint256 => bytes)`).
    let mut preimage = Vec::with_capacity(suffix.len() + 32);
    preimage.extend_from_slice(suffix);
    preimage.extend_from_slice(root.as_bytes());
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
        // On-wire capacity limit: `own_entry_count` and `count` are u8, so a
        // node holds at most 255 slots → 2T-1 ≤ 255 → T ≤ 128.
        assert!(
            T <= 128,
            "OrderedIndex: T > 128 exceeds u8 count capacity (2T-1 > 255)"
        );
        let root_key = StorageKey(pvm_contract_types::keccak256(namespace));
        let root_cell_key = derive_cell_key(&root_key, b"root");
        let next_id_cell_key = derive_cell_key(&root_key, b"next_id");
        let next_nonce_cell_key = derive_cell_key(&root_key, b"next_nonce");
        Self {
            root_key,
            root_cell_key,
            next_id_cell_key,
            next_nonce_cell_key,
            _marker: PhantomData,
        }
    }

    fn cell_lazy(&self, host: &Host, key: StorageKey) -> Lazy<u64> {
        // SAFETY: `Lazy::new` is `unsafe` only because it bypasses the
        // `#[storage]` layout walker; its contract is that no two `Lazy`s
        // claim overlapping storage keys. The two admin cell keys
        // (next_id/next_nonce) are distinct `keccak256(suffix ++ root)`
        // values (Solidity `mapping(string => bytes)` entries, distinct
        // suffixes → distinct preimages → distinct hashes), node bodies
        // live under `NodeId`-derived keys (`keccak256(pad32(id) ++ root)`,
        // a disjoint `mapping(uint256 => bytes)` namespace), and the root
        // node body lives at the `b"root"` cell key as a *bytes* value
        // (`storage_*_bytes`, never a `Lazy<u64>`) — so nothing overlaps.
        // `offset` is 0 (full-slot u64).
        unsafe { Lazy::<u64>::new(key, 0, host.clone()) }
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
            Ok(n) => n,
            Err(_) => panic!("OrderedIndexCorruptNode"),
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

    fn alloc_nonce(&self, host: &Host) -> Nonce {
        let n = self.next_nonce_cell_lazy(host).get();
        let next = n.checked_add(1).expect("OrderedIndexNonceOverflow");
        self.next_nonce_cell_lazy(host).set(&next);
        Nonce(n)
    }

    fn load_root(&self, host: &Host) -> Option<Node<K, V>> {
        let bytes = storage_get_bytes(host, self.root_cell_key.as_bytes()).unwrap_or_default();
        if bytes.is_empty() {
            return None;
        }
        match Node::decode(&bytes) {
            Ok(n) => Some(n),
            Err(_) => panic!("OrderedIndexCorruptNode"),
        }
    }

    fn store_root(&self, host: &Host, node: &Node<K, V>) {
        self.assert_node_shape(node);
        self.assert_node_size(node);
        let bytes = node.encode();
        storage_set_bytes(host, self.root_cell_key.as_bytes(), &bytes);
    }

    fn clear_root(&self, host: &Host) {
        storage_clear_value(host, self.root_cell_key.as_bytes());
    }

    fn child_mirror(&self, host: &Host, id: NodeId) -> ChildRef {
        let child = self.load_node(host, id);
        ChildRef {
            id,
            subtree_count: SubtreeCount(child.subtree_count()),
            entry_count: EntryCount(child.slot_count() as u32),
        }
    }

    fn refresh_child_mirrors(&self, host: &Host, node: &mut Node<K, V>, child_idx: usize) {
        if let Node::Internal { children, .. } = node {
            let id = children[child_idx].id;
            children[child_idx] = self.child_mirror(host, id);
        }
    }

    fn assert_node_shape(&self, node: &Node<K, V>) {
        if node.slot_count() > u8::MAX as usize {
            panic!("OrderedIndexNodeTooManyEntries");
        }
        if let Node::Internal {
            separators,
            children,
        } = node
            && children.len() != separators.len() + 1
        {
            panic!("OrderedIndexBadChildMirrors");
        }
        match node {
            Node::Leaf { entries, .. } => {
                for i in 1..entries.len() {
                    let prev = &entries[i - 1];
                    let curr = &entries[i];
                    if prev.key > curr.key || (prev.key == curr.key && prev.nonce.0 >= curr.nonce.0)
                    {
                        panic!("OrderedIndexUnsortedNode");
                    }
                }
            }
            Node::Internal { separators, .. } => {
                for i in 1..separators.len() {
                    let prev = &separators[i - 1];
                    let curr = &separators[i];
                    if prev.key > curr.key || (prev.key == curr.key && prev.nonce.0 >= curr.nonce.0)
                    {
                        panic!("OrderedIndexUnsortedNode");
                    }
                }
            }
        }
    }

    fn assert_node_size(&self, node: &Node<K, V>) {
        if node.encode().len() > MAX_STORAGE_VALUE_BYTES {
            panic!("OrderedIndexNodeTooLarge");
        }
    }

    pub fn len(&self, host: &Host) -> u64 {
        match self.load_root(host) {
            None => 0,
            Some(root) => root.subtree_count(),
        }
    }

    pub fn is_empty(&self, host: &Host) -> bool {
        self.len(host) == 0
    }

    pub fn insert(&self, host: &Host, key: &K, value: &V) -> u64 {
        let nonce = self.alloc_nonce(host);
        match self.load_root(host) {
            None => {
                let mut root = Node::<K, V>::leaf();
                if let Node::Leaf { entries, .. } = &mut root {
                    entries.push(LeafEntry {
                        key: key.clone(),
                        nonce,
                        value: value.clone(),
                    });
                }
                self.store_root(host, &root);
            }
            Some(mut root) => {
                self.insert_into_node(host, &mut root, key, nonce, value);
                if root.encode().len() > MAX_STORAGE_VALUE_BYTES {
                    let split = self.split_into_new_node(host, root);
                    let new_root = Node::Internal {
                        separators: alloc::vec![Separator {
                            key: split.sep_key,
                            nonce: split.sep_nonce,
                        }],
                        children: alloc::vec![
                            ChildRef {
                                id: split.left_id,
                                subtree_count: SubtreeCount(split.left_subtree_count),
                                entry_count: EntryCount(split.left_entry_count),
                            },
                            ChildRef {
                                id: split.right_id,
                                subtree_count: SubtreeCount(split.right_subtree_count),
                                entry_count: EntryCount(split.right_entry_count),
                            },
                        ],
                    };
                    self.store_root(host, &new_root);
                } else {
                    self.store_root(host, &root);
                }
            }
        }
        nonce.0
    }

    fn insert_into_node(
        &self,
        host: &Host,
        node: &mut Node<K, V>,
        key: &K,
        nonce: Nonce,
        value: &V,
    ) {
        match node {
            Node::Leaf { entries, .. } => {
                let pos = lower_bound_entry_leaf(entries, key, nonce);
                entries.insert(
                    pos,
                    LeafEntry {
                        key: key.clone(),
                        nonce,
                        value: value.clone(),
                    },
                );
            }
            Node::Internal {
                separators,
                children,
            } => {
                let child_idx = lower_bound_entry_sep(separators, key, nonce);
                let child_id = children[child_idx].id;
                match self.insert_rec(host, child_id, key, nonce, value) {
                    None => {
                        children[child_idx] = self.child_mirror(host, child_id);
                    }
                    Some(split) => {
                        separators.insert(
                            child_idx,
                            Separator {
                                key: split.sep_key,
                                nonce: split.sep_nonce,
                            },
                        );
                        children[child_idx] = ChildRef {
                            id: child_id,
                            subtree_count: SubtreeCount(split.left_subtree_count),
                            entry_count: EntryCount(split.left_entry_count),
                        };
                        children.insert(
                            child_idx + 1,
                            ChildRef {
                                id: split.right_id,
                                subtree_count: SubtreeCount(split.right_subtree_count),
                                entry_count: EntryCount(split.right_entry_count),
                            },
                        );
                    }
                }
            }
        }
    }

    fn split_into_new_node(&self, host: &Host, node: Node<K, V>) -> RootSplit<K> {
        match node {
            Node::Leaf { entries, next } => {
                let cut = self.leaf_cut(&entries, next);
                let right_entries = entries[cut..].to_vec();
                let left_entries = entries[..cut].to_vec();
                let sep_key = right_entries[0].key.clone();
                let sep_nonce = right_entries[0].nonce;

                let right = Node::Leaf {
                    entries: right_entries,
                    next,
                };
                let right_id = self.alloc_node(host, &right);
                let left = Node::Leaf {
                    entries: left_entries,
                    next: Some(right_id),
                };
                let left_count = left.subtree_count();
                let left_entry_count = left.slot_count() as u32;
                let right_count = right.subtree_count();
                let right_entry_count = right.slot_count() as u32;
                let left_id = self.alloc_node(host, &left);
                RootSplit {
                    sep_key,
                    sep_nonce,
                    left_id,
                    right_id,
                    left_subtree_count: left_count,
                    right_subtree_count: right_count,
                    left_entry_count,
                    right_entry_count,
                }
            }
            Node::Internal {
                separators,
                children,
            } => {
                let cut = self.internal_cut(&separators, &children);
                let median = separators[cut].clone();
                let left = Node::Internal {
                    separators: separators[..cut].to_vec(),
                    children: children[..=cut].to_vec(),
                };
                let right = Node::Internal {
                    separators: separators[cut + 1..].to_vec(),
                    children: children[cut + 1..].to_vec(),
                };
                let left_count = left.subtree_count();
                let left_entry_count = left.slot_count() as u32;
                let right_count = right.subtree_count();
                let right_entry_count = right.slot_count() as u32;
                let left_id = self.alloc_node(host, &left);
                let right_id = self.alloc_node(host, &right);
                RootSplit {
                    sep_key: median.key,
                    sep_nonce: median.nonce,
                    left_id,
                    right_id,
                    left_subtree_count: left_count,
                    right_subtree_count: right_count,
                    left_entry_count,
                    right_entry_count,
                }
            }
        }
    }

    pub fn get_first(&self, host: &Host, key: &K) -> Option<V> {
        self.first_entry_for(host, key).map(|(_, value)| value)
    }

    /// Descend once to the candidate leaf, then walk the leaf next-link chain
    /// forward to the first entry with `key`. Routing by separator can land
    /// one leaf before the leftmost occurrence (when the key is a leaf's
    /// first key, or spans several leaves); the forward walk recovers it
    /// without re-descending. Stops as soon as a key strictly greater than
    /// `key` is seen.
    fn first_entry_for(&self, host: &Host, key: &K) -> Option<(Nonce, V)> {
        let root = self.load_root(host)?;
        let mut node = self.descend_to_first_leaf(host, root, Bound::Included(key));
        loop {
            let Node::Leaf { entries, next } = &node else {
                panic!("OrderedIndexCorruptNode");
            };
            let pos = lower_bound_key(entries_keys_leaf(entries), key);
            if pos < entries.len() {
                let e = &entries[pos];
                if e.key == *key {
                    return Some((e.nonce, e.value.clone()));
                }
                return None;
            }
            let next_id = match next {
                Some(next_id) => *next_id,
                None => return None,
            };
            node = self.load_node(host, next_id);
        }
    }

    pub fn remove_by_nonce(&self, host: &Host, key: &K, nonce: u64) -> Option<V> {
        let mut root = self.load_root(host)?;
        let removed = self.remove_in_node(host, &mut root, key, Nonce(nonce));
        if removed.is_some() {
            match &root {
                Node::Leaf { entries, .. } => {
                    if entries.is_empty() {
                        self.clear_root(host);
                    } else {
                        self.store_root(host, &root);
                    }
                }
                Node::Internal {
                    separators,
                    children,
                } => {
                    if separators.is_empty() {
                        let sole = children[0].id;
                        let pulled = self.load_node(host, sole);
                        self.free_node(host, sole);
                        self.store_root(host, &pulled);
                    } else {
                        self.store_root(host, &root);
                    }
                }
            }
        }
        removed
    }

    fn remove_in_node(&self, host: &Host, node: &mut Node<K, V>, k: &K, nonce: Nonce) -> Option<V> {
        match node {
            Node::Leaf { entries, .. } => {
                let pos = lower_bound_entry_leaf(entries, k, nonce);
                if pos < entries.len() && entries[pos].key == *k && entries[pos].nonce == nonce {
                    Some(entries.remove(pos).value)
                } else {
                    None
                }
            }
            Node::Internal { separators, .. } => {
                let pos = upper_bound_entry_sep(separators, k, nonce);
                let descend = self.descend_prepared(host, node, pos);
                let child_id = node.children()[descend].id;
                let result = self.remove_from(host, child_id, k, nonce);
                if result.is_some() {
                    self.refresh_child_mirrors(host, node, descend);
                }
                result
            }
        }
    }

    pub fn remove_first(&self, host: &Host, key: &K) -> Option<V> {
        let nonce = self.find_first_nonce(host, key)?;
        self.remove_by_nonce(host, key, nonce.0)
    }

    pub fn remove(&self, host: &Host, key: &K, value: &V) -> bool
    where
        V: PartialEq,
    {
        match self.find_nonce_for(host, key, value) {
            Some(n) => self.remove_by_nonce(host, key, n.0).is_some(),
            None => false,
        }
    }

    pub fn select(&self, host: &Host, rank: u64) -> Option<(K, V)> {
        self.select_with_nonce(host, rank).map(|(k, _, v)| (k, v))
    }

    pub fn rank_of_key(&self, host: &Host, key: &K) -> u64 {
        let mut node = match self.load_root(host) {
            Some(node) => node,
            None => return 0,
        };
        let mut rank: u64 = 0;
        loop {
            match &node {
                Node::Leaf { entries, .. } => {
                    let pos = lower_bound_key(entries_keys_leaf(entries), key);
                    return rank + pos as u64;
                }
                Node::Internal {
                    separators,
                    children,
                } => {
                    let pos = lower_bound_key_sep(separators, key);
                    for c in &children[..pos] {
                        rank += c.subtree_count.0;
                    }
                    let id = children[pos].id;
                    node = self.load_node(host, id);
                }
            }
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
        let Some(root) = self.load_root(host) else {
            return out;
        };

        // Descend ONCE to the first in-range leaf, following separators only.
        let mut node = self.descend_to_first_leaf(host, root, from);
        let mut skipped: u64 = 0;
        loop {
            let Node::Leaf { entries, next } = &node else {
                // Directory corruption: descent must end at a leaf.
                panic!("OrderedIndexCorruptNode");
            };
            let start = match from {
                Bound::Unbounded => 0,
                Bound::Included(k) => lower_bound_key(entries_keys_leaf(entries), k),
                Bound::Excluded(k) => upper_bound_key(entries_keys_leaf(entries), k),
            };
            for e in &entries[start..] {
                let above = match to {
                    Bound::Unbounded => false,
                    Bound::Included(k) => e.key > *k,
                    Bound::Excluded(k) => e.key >= *k,
                };
                if above {
                    return out;
                }
                if skipped < offset {
                    skipped += 1;
                } else {
                    out.push((e.key.clone(), e.value.clone()));
                    if out.len() as u64 == limit {
                        return out;
                    }
                }
            }
            let next_id = match next {
                Some(next_id) => *next_id,
                None => return out,
            };
            node = self.load_node(host, next_id);
        }
    }

    /// Follow separators from `id` down to the leaf that holds (or would
    /// hold) the first key satisfying `from`, returning the already-loaded
    /// leaf node (so the caller never re-reads it) and its id. Touches one
    /// node per level — the depth — and zero leaves beyond the first.
    fn descend_to_first_leaf(&self, host: &Host, start: Node<K, V>, from: Bound<&K>) -> Node<K, V> {
        let mut node = start;
        loop {
            match &node {
                Node::Leaf { .. } => return node,
                Node::Internal {
                    separators,
                    children,
                } => {
                    let pos = match from {
                        Bound::Unbounded => 0,
                        Bound::Included(k) => lower_bound_key_sep(separators, k),
                        Bound::Excluded(k) => upper_bound_key_sep(separators, k),
                    };
                    let id = children[pos].id;
                    node = self.load_node(host, id);
                }
            }
        }
    }

    fn insert_rec(
        &self,
        host: &Host,
        node_id: NodeId,
        key: &K,
        nonce: Nonce,
        value: &V,
    ) -> Option<ChildSplit<K>> {
        let mut node = self.load_node(host, node_id);

        match &mut node {
            Node::Leaf { entries, .. } => {
                let pos = lower_bound_entry_leaf(entries, key, nonce);
                entries.insert(
                    pos,
                    LeafEntry {
                        key: key.clone(),
                        nonce,
                        value: value.clone(),
                    },
                );
            }
            Node::Internal {
                separators,
                children,
            } => {
                let child_idx = lower_bound_entry_sep(separators, key, nonce);
                let child_id = children[child_idx].id;
                match self.insert_rec(host, child_id, key, nonce, value) {
                    None => {
                        children[child_idx] = self.child_mirror(host, child_id);
                    }
                    Some(split) => {
                        separators.insert(
                            child_idx,
                            Separator {
                                key: split.sep_key,
                                nonce: split.sep_nonce,
                            },
                        );
                        children[child_idx] = ChildRef {
                            id: child_id,
                            subtree_count: SubtreeCount(split.left_subtree_count),
                            entry_count: EntryCount(split.left_entry_count),
                        };
                        children.insert(
                            child_idx + 1,
                            ChildRef {
                                id: split.right_id,
                                subtree_count: SubtreeCount(split.right_subtree_count),
                                entry_count: EntryCount(split.right_entry_count),
                            },
                        );
                    }
                }
            }
        }

        if node.encode().len() > MAX_STORAGE_VALUE_BYTES {
            self.split_node(host, node_id, node)
        } else {
            self.store_node(host, node_id, &node);
            None
        }
    }

    /// Split an over-full node, store the left half in place, allocate the
    /// right half, and return the separator + child mirrors to the parent.
    /// Leaf split COPIES the right half's first key up (B+ — the median key
    /// stays in the right leaf too) AND rewires the next-links. Internal
    /// split PUSHES the median separator up (it is removed from both halves).
    fn split_node(&self, host: &Host, node_id: NodeId, node: Node<K, V>) -> Option<ChildSplit<K>> {
        match node {
            Node::Leaf { entries, next } => {
                let cut = self.leaf_cut(&entries, next);
                let right_entries = entries[cut..].to_vec();
                let left_entries = entries[..cut].to_vec();
                let sep_key = right_entries[0].key.clone();
                let sep_nonce = right_entries[0].nonce;

                // Rewire: new right inherits old.next; left points to new right.
                let right = Node::Leaf {
                    entries: right_entries,
                    next,
                };
                let right_id = self.alloc_node(host, &right);
                let left = Node::Leaf {
                    entries: left_entries,
                    next: Some(right_id),
                };
                let left_count = left.subtree_count();
                let left_entry_count = left.slot_count() as u32;
                let right_count = right.subtree_count();
                let right_entry_count = right.slot_count() as u32;
                self.store_node(host, node_id, &left);
                Some(ChildSplit {
                    sep_key,
                    sep_nonce,
                    right_id,
                    left_subtree_count: left_count,
                    right_subtree_count: right_count,
                    left_entry_count,
                    right_entry_count,
                })
            }
            Node::Internal {
                separators,
                children,
            } => {
                let cut = self.internal_cut(&separators, &children);
                let median = separators[cut].clone();
                let left = Node::Internal {
                    separators: separators[..cut].to_vec(),
                    children: children[..=cut].to_vec(),
                };
                let right = Node::Internal {
                    separators: separators[cut + 1..].to_vec(),
                    children: children[cut + 1..].to_vec(),
                };
                let left_count = left.subtree_count();
                let left_entry_count = left.slot_count() as u32;
                let right_count = right.subtree_count();
                let right_entry_count = right.slot_count() as u32;
                self.store_node(host, node_id, &left);
                let right_id = self.alloc_node(host, &right);
                Some(ChildSplit {
                    sep_key: median.key,
                    sep_nonce: median.nonce,
                    right_id,
                    left_subtree_count: left_count,
                    right_subtree_count: right_count,
                    left_entry_count,
                    right_entry_count,
                })
            }
        }
    }

    /// Choose a leaf split point. The corpus is overwhelmingly append-mostly
    /// (records arrive in key order), so a pack-left bias keeps every closed
    /// left leaf nearly full: this halves the leaf count versus an even
    /// split and lets a typical result page fit one leaf, cutting the
    /// leaf-walk reads. The cut is the largest index where the left half
    /// still fits the byte cap, with the right half keeping at least the
    /// min-degree floor (T-1).
    fn leaf_cut(&self, entries: &[LeafEntry<K, V>], next: Option<NodeId>) -> usize {
        let m = entries.len();
        let lo = T - 1;
        let hi = m - (T - 1);
        let fits = |cut: usize| -> bool {
            let left = Node::Leaf {
                entries: entries[..cut].to_vec(),
                next: Some(NodeId(u64::MAX)),
            };
            let right = Node::Leaf {
                entries: entries[cut..].to_vec(),
                next,
            };
            left.encode().len() <= MAX_STORAGE_VALUE_BYTES
                && right.encode().len() <= MAX_STORAGE_VALUE_BYTES
        };
        let mut cut = hi;
        loop {
            if fits(cut) {
                return cut;
            }
            if cut > lo {
                cut -= 1;
            } else {
                panic!("OrderedIndexNoByteSplit: leaf {} entries at T={}", m, T);
            }
        }
    }

    /// Choose an internal split point (the pushed-up median index) keeping
    /// both halves within cap and min-degree.
    fn internal_cut(&self, separators: &[Separator<K>], children: &[ChildRef]) -> usize {
        let m = separators.len();
        let lo = T - 1;
        let hi = m - T; // right keeps m - cut - 1 separators >= T-1 → cut <= m-T
        let mut cut = (m / 2).clamp(lo, hi);
        loop {
            let left = Node::<K, V>::Internal {
                separators: separators[..cut].to_vec(),
                children: children[..=cut].to_vec(),
            };
            let right = Node::<K, V>::Internal {
                separators: separators[cut + 1..].to_vec(),
                children: children[cut + 1..].to_vec(),
            };
            let left_ok = left.encode().len() <= MAX_STORAGE_VALUE_BYTES;
            let right_ok = right.encode().len() <= MAX_STORAGE_VALUE_BYTES;
            if left_ok && right_ok {
                return cut;
            }
            if !left_ok && cut > lo {
                cut -= 1;
            } else if !right_ok && cut < hi {
                cut += 1;
            } else {
                panic!("OrderedIndexNoByteSplit: internal {} seps at T={}", m, T);
            }
        }
    }

    fn remove_from(&self, host: &Host, node_id: NodeId, k: &K, nonce: Nonce) -> Option<V> {
        let mut node = self.load_node(host, node_id);
        match &mut node {
            Node::Leaf { entries, .. } => {
                let pos = lower_bound_entry_leaf(entries, k, nonce);
                if pos < entries.len() && entries[pos].key == *k && entries[pos].nonce == nonce {
                    let removed = entries.remove(pos).value;
                    self.store_node(host, node_id, &node);
                    Some(removed)
                } else {
                    None
                }
            }
            Node::Internal { separators, .. } => {
                let pos = upper_bound_entry_sep(separators, k, nonce);
                // In a B+ tree data lives only in leaves: always descend.
                let descend = self.descend_prepared(host, &mut node, pos);
                let child_id = node.children()[descend].id;
                self.store_node(host, node_id, &node);
                let result = self.remove_from(host, child_id, k, nonce);
                if result.is_some() {
                    self.refresh_child_mirrors(host, &mut node, descend);
                    self.store_node(host, node_id, &node);
                }
                result
            }
        }
    }

    fn descend_prepared(&self, host: &Host, node: &mut Node<K, V>, pos: usize) -> usize {
        if node.children()[pos].entry_count.0 >= T as u32 {
            return pos;
        }
        let has_left = pos > 0;
        let has_right = pos + 1 < node.children().len();

        if has_left && node.children()[pos - 1].entry_count.0 >= T as u32 {
            self.borrow_from_left(host, node, pos);
            pos
        } else if has_right && node.children()[pos + 1].entry_count.0 >= T as u32 {
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
        let left_id = node.children()[pos - 1].id;
        let child_id = node.children()[pos].id;
        let mut left = self.load_node(host, left_id);
        let mut child = self.load_node(host, child_id);

        let new_sep = match (&mut left, &mut child) {
            // Leaf borrow: move the last left entry into the child; the new
            // separator is the moved entry's key/nonce (B+ — separator mirrors
            // the right subtree's first key). Leaves keep their next-links.
            (
                Node::Leaf {
                    entries: left_entries,
                    ..
                },
                Node::Leaf {
                    entries: child_entries,
                    ..
                },
            ) => {
                let moved = match left_entries.pop() {
                    Some(e) => e,
                    None => panic!("OrderedIndexBorrowLeftEmpty"),
                };
                let sep = Separator {
                    key: moved.key.clone(),
                    nonce: moved.nonce,
                };
                child_entries.insert(0, moved);
                sep
            }
            // Internal borrow: rotate through the parent separator and move
            // the last left child across.
            (
                Node::Internal {
                    separators: left_seps,
                    children: left_children,
                },
                Node::Internal {
                    separators: child_seps,
                    children: child_children,
                },
            ) => {
                let parent_sep = match node {
                    Node::Internal { separators, .. } => separators[pos - 1].clone(),
                    Node::Leaf { .. } => panic!("OrderedIndexBorrowOnLeafParent"),
                };
                let new_sep = match left_seps.pop() {
                    Some(s) => s,
                    None => panic!("OrderedIndexBorrowLeftEmpty"),
                };
                child_seps.insert(0, parent_sep);
                let moved = match left_children.pop() {
                    Some(c) => c,
                    None => panic!("OrderedIndexMissingLeftChild"),
                };
                child_children.insert(0, moved);
                new_sep
            }
            _ => panic!("OrderedIndexMixedSiblings"),
        };

        let left_mirror = ChildRef {
            id: left_id,
            subtree_count: SubtreeCount(left.subtree_count()),
            entry_count: EntryCount(left.slot_count() as u32),
        };
        let child_mirror = ChildRef {
            id: child_id,
            subtree_count: SubtreeCount(child.subtree_count()),
            entry_count: EntryCount(child.slot_count() as u32),
        };
        if let Node::Internal {
            separators,
            children,
        } = node
        {
            separators[pos - 1] = new_sep;
            children[pos - 1] = left_mirror;
            children[pos] = child_mirror;
        }

        self.store_node(host, left_id, &left);
        self.store_node(host, child_id, &child);
    }

    fn borrow_from_right(&self, host: &Host, node: &mut Node<K, V>, pos: usize) {
        let child_id = node.children()[pos].id;
        let right_id = node.children()[pos + 1].id;
        let mut child = self.load_node(host, child_id);
        let mut right = self.load_node(host, right_id);

        let new_sep = match (&mut child, &mut right) {
            (
                Node::Leaf {
                    entries: child_entries,
                    ..
                },
                Node::Leaf {
                    entries: right_entries,
                    ..
                },
            ) => {
                let moved = right_entries.remove(0);
                child_entries.push(moved);
                // New separator mirrors the right leaf's new first key.
                Separator {
                    key: right_entries[0].key.clone(),
                    nonce: right_entries[0].nonce,
                }
            }
            (
                Node::Internal {
                    separators: child_seps,
                    children: child_children,
                },
                Node::Internal {
                    separators: right_seps,
                    children: right_children,
                },
            ) => {
                let parent_sep = match node {
                    Node::Internal { separators, .. } => separators[pos].clone(),
                    Node::Leaf { .. } => panic!("OrderedIndexBorrowOnLeafParent"),
                };
                let new_sep = right_seps.remove(0);
                child_seps.push(parent_sep);
                let moved = right_children.remove(0);
                child_children.push(moved);
                new_sep
            }
            _ => panic!("OrderedIndexMixedSiblings"),
        };

        let child_mirror = ChildRef {
            id: child_id,
            subtree_count: SubtreeCount(child.subtree_count()),
            entry_count: EntryCount(child.slot_count() as u32),
        };
        let right_mirror = ChildRef {
            id: right_id,
            subtree_count: SubtreeCount(right.subtree_count()),
            entry_count: EntryCount(right.slot_count() as u32),
        };
        if let Node::Internal {
            separators,
            children,
        } = node
        {
            separators[pos] = new_sep;
            children[pos] = child_mirror;
            children[pos + 1] = right_mirror;
        }

        self.store_node(host, child_id, &child);
        self.store_node(host, right_id, &right);
    }

    fn merge_children(&self, host: &Host, node: &mut Node<K, V>, pos: usize) {
        let left_id = node.children()[pos].id;
        let right_id = node.children()[pos + 1].id;
        let mut left = self.load_node(host, left_id);
        let right = self.load_node(host, right_id);

        match (&mut left, right) {
            // Leaf merge: concatenate entries, inherit the right leaf's
            // next-link, and DROP the parent separator (it carries no payload
            // in B+).
            (
                Node::Leaf {
                    entries: left_entries,
                    next: left_next,
                },
                Node::Leaf {
                    entries: right_entries,
                    next: right_next,
                },
            ) => {
                left_entries.extend(right_entries);
                *left_next = right_next;
                if let Node::Internal { separators, .. } = node {
                    separators.remove(pos);
                }
            }
            // Internal merge: pull the parent separator DOWN between the two
            // halves (it routes a real subtree boundary).
            (
                Node::Internal {
                    separators: left_seps,
                    children: left_children,
                },
                Node::Internal {
                    separators: right_seps,
                    children: right_children,
                },
            ) => {
                let parent_sep = match node {
                    Node::Internal { separators, .. } => separators.remove(pos),
                    Node::Leaf { .. } => panic!("OrderedIndexMergeOnLeafParent"),
                };
                left_seps.push(parent_sep);
                left_seps.extend(right_seps);
                left_children.extend(right_children);
            }
            _ => panic!("OrderedIndexMixedSiblings"),
        }

        let merged = ChildRef {
            id: left_id,
            subtree_count: SubtreeCount(left.subtree_count()),
            entry_count: EntryCount(left.slot_count() as u32),
        };
        if let Node::Internal { children, .. } = node {
            children.remove(pos + 1);
            children[pos] = merged;
        }

        self.store_node(host, left_id, &left);
        self.free_node(host, right_id);
    }

    fn find_first_nonce(&self, host: &Host, k: &K) -> Option<Nonce> {
        self.first_entry_for(host, k).map(|(nonce, _)| nonce)
    }

    fn find_nonce_for(&self, host: &Host, k: &K, v: &V) -> Option<Nonce>
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

    fn select_with_nonce(&self, host: &Host, mut rank: u64) -> Option<(K, Nonce, V)> {
        let mut node = self.load_root(host)?;
        loop {
            if rank >= node.subtree_count() {
                return None;
            }
            let next_id = match &node {
                Node::Leaf { entries, .. } => {
                    let e = &entries[rank as usize];
                    return Some((e.key.clone(), e.nonce, e.value.clone()));
                }
                Node::Internal { children, .. } => {
                    let mut descended: Option<NodeId> = None;
                    for c in children {
                        if rank < c.subtree_count.0 {
                            descended = Some(c.id);
                            break;
                        }
                        rank -= c.subtree_count.0;
                    }
                    descended?
                }
            };
            node = self.load_node(host, next_id);
        }
    }

    #[cfg(test)]
    fn walk_all_nodes(&self, host: &Host, visit: &mut dyn FnMut(&Node<K, V>)) {
        if let Some(root) = self.load_root(host) {
            visit(&root);
            for child in root.children() {
                self.walk_node(host, child.id, visit);
            }
        }
    }

    #[cfg(test)]
    fn walk_node(&self, host: &Host, id: NodeId, visit: &mut dyn FnMut(&Node<K, V>)) {
        let node = self.load_node(host, id);
        visit(&node);
        for child in node.children() {
            self.walk_node(host, child.id, visit);
        }
    }
}

struct ChildSplit<K: SolEncode + SolDecode + Clone + CompactCodec> {
    sep_key: K,
    sep_nonce: Nonce,
    right_id: NodeId,
    left_subtree_count: u64,
    right_subtree_count: u64,
    left_entry_count: u32,
    right_entry_count: u32,
}

struct RootSplit<K: SolEncode + SolDecode + Clone + CompactCodec> {
    sep_key: K,
    sep_nonce: Nonce,
    left_id: NodeId,
    right_id: NodeId,
    left_subtree_count: u64,
    right_subtree_count: u64,
    left_entry_count: u32,
    right_entry_count: u32,
}

fn entries_keys_leaf<
    K: SolEncode + SolDecode + Clone + CompactCodec,
    V: SolEncode + SolDecode + Clone + CompactCodec,
>(
    entries: &[LeafEntry<K, V>],
) -> &[LeafEntry<K, V>] {
    entries
}

fn lower_bound_key<
    K: SolEncode + SolDecode + Clone + CompactCodec + Ord,
    V: SolEncode + SolDecode + Clone + CompactCodec,
>(
    entries: &[LeafEntry<K, V>],
    k: &K,
) -> usize {
    entries.partition_point(|e| e.key < *k)
}

fn upper_bound_key<
    K: SolEncode + SolDecode + Clone + CompactCodec + Ord,
    V: SolEncode + SolDecode + Clone + CompactCodec,
>(
    entries: &[LeafEntry<K, V>],
    k: &K,
) -> usize {
    entries.partition_point(|e| e.key <= *k)
}

fn lower_bound_entry_leaf<
    K: SolEncode + SolDecode + Clone + CompactCodec + Ord,
    V: SolEncode + SolDecode + Clone + CompactCodec,
>(
    entries: &[LeafEntry<K, V>],
    k: &K,
    nonce: Nonce,
) -> usize {
    entries.partition_point(|e| e.key < *k || (e.key == *k && e.nonce.0 < nonce.0))
}

fn lower_bound_key_sep<K: SolEncode + SolDecode + Clone + CompactCodec + Ord>(
    separators: &[Separator<K>],
    k: &K,
) -> usize {
    separators.partition_point(|s| s.key < *k)
}

fn upper_bound_key_sep<K: SolEncode + SolDecode + Clone + CompactCodec + Ord>(
    separators: &[Separator<K>],
    k: &K,
) -> usize {
    separators.partition_point(|s| s.key <= *k)
}

fn lower_bound_entry_sep<K: SolEncode + SolDecode + Clone + CompactCodec + Ord>(
    separators: &[Separator<K>],
    k: &K,
    nonce: Nonce,
) -> usize {
    separators.partition_point(|s| s.key < *k || (s.key == *k && s.nonce.0 < nonce.0))
}

fn upper_bound_entry_sep<K: SolEncode + SolDecode + Clone + CompactCodec + Ord>(
    separators: &[Separator<K>],
    k: &K,
    nonce: Nonce,
) -> usize {
    separators.partition_point(|s| s.key < *k || (s.key == *k && s.nonce.0 <= nonce.0))
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::collections::BTreeMap;
    use alloc::rc::Rc;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::ops::Bound;
    use proptest::prelude::*;
    use pvm_contract_types::{Host, MockHostBuilder};

    use super::{MAX_STORAGE_VALUE_BYTES, Node, NodeDecodeError, NodeId, OrderedIndex, StorageKey};

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

    /// OrderedIndex must use Solidity-compatible key derivation, the repo's
    /// headline invariant (lib.rs: "Solidity-compatible slot layout"), which
    /// Lazy/Mapping/StorageVec all prove via `cast index` cross-checks.
    /// OrderedIndex was the lone outlier (bespoke `keccak256(ns ++ 0x00 ++ suf)`);
    /// this test pins it to the Solidity scheme two ways — the exact Solidity
    /// preimage computed independently, and the crate's cast-validated helper.
    #[test]
    fn ordered_index_cell_and_node_keys_are_solidity_layout() {
        let host = host();
        let idx = index(&host);
        let root: &[u8; 32] = idx.root_key.as_bytes();

        let cells: [(&str, &[u8], &StorageKey); 3] = [
            ("root", b"root" as &[u8], &idx.root_cell_key),
            ("next_id", b"next_id" as &[u8], &idx.next_id_cell_key),
            (
                "next_nonce",
                b"next_nonce" as &[u8],
                &idx.next_nonce_cell_key,
            ),
        ];
        for (name, suffix, cell) in cells {
            // Solidity mapping(string => bytes): keccak256(suffix ++ pad32(root))
            let mut preimage = Vec::with_capacity(suffix.len() + 32);
            preimage.extend_from_slice(suffix);
            preimage.extend_from_slice(root);
            let manual = pvm_contract_types::keccak256(&preimage);
            let helper = crate::storage_derive_key_unpadded(&host, root, suffix);
            assert_eq!(
                cell.0, manual,
                "cell `{name}` must match the Solidity mapping(string) preimage"
            );
            assert_eq!(
                cell.0, helper,
                "cell `{name}` must match the cast-validated storage_derive_key_unpadded"
            );
        }

        // Solidity mapping(uint256 => bytes): keccak256(pad32(id) ++ pad32(root))
        let node7 = idx.node_key(&host, NodeId(7));
        let mut padded_id = [0u8; 32];
        padded_id[24..32].copy_from_slice(&7u64.to_be_bytes());
        let mut preimage = [0u8; 64];
        preimage[0..32].copy_from_slice(&padded_id);
        preimage[32..64].copy_from_slice(root);
        assert_eq!(
            node7.0,
            pvm_contract_types::keccak256(&preimage),
            "node key must match the Solidity mapping(uint256) preimage"
        );
        assert_eq!(
            node7.0,
            crate::storage_derive_key(&host, root, &padded_id),
            "node key must match the cast-validated storage_derive_key helper"
        );
    }

    #[test]
    fn node_decode_distinguishes_failure_variants() {
        let host = host();
        let idx = index(&host);
        for i in 0..120u64 {
            idx.insert(&host, &alloc::format!("user{i:04}"), &i);
        }
        let internal = idx.load_root(&host).expect("non-empty root");
        assert!(
            !internal.is_leaf(),
            "120 inserts at T=2 must yield an internal root"
        );
        let leaf = idx.load_node(&host, internal.children()[0].id);
        assert!(leaf.is_leaf());

        for node in [&internal, &leaf] {
            let bytes = node.encode();
            assert!(
                Node::<String, u64>::decode(&bytes).is_ok(),
                "roundtrip must decode"
            );

            let mut trailing = bytes.clone();
            trailing.push(0);
            assert_eq!(
                Node::<String, u64>::decode(&trailing).err(),
                Some(NodeDecodeError::TrailingBytes),
                "a byte past the encoded node is TrailingBytes",
            );

            for cut in 0..bytes.len() {
                assert!(
                    Node::<String, u64>::decode(&bytes[..cut]).is_err(),
                    "every truncation (len {cut}) must be rejected, not silently decoded",
                );
            }
        }

        assert_eq!(
            Node::<String, u64>::decode(&[]).err(),
            Some(NodeDecodeError::Truncated),
            "an empty buffer is Truncated",
        );
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
            // Structural invariants on the enum shape: data lives only in
            // leaves; an internal node has exactly separators+1 children.
            idx.walk_all_nodes(&host, &mut |node: &Node<String, u64>| {
                assert!(node.slot_count() <= u8::MAX as usize, "node too many slots");
                assert!(
                    node.encode().len() <= MAX_STORAGE_VALUE_BYTES,
                    "node too large: {} bytes",
                    node.encode().len()
                );
                match node {
                    Node::Internal { separators, children } => {
                        assert_eq!(children.len(), separators.len() + 1,
                            "internal node must have separators+1 children");
                    }
                    Node::Leaf { .. } => {}
                }
            });
            if let Some(root) = idx.load_root(&host) {
                let mut depths: Vec<usize> = Vec::new();
                walk_depths_node(&idx, &host, &root, 0, &mut depths);
                if !depths.is_empty() {
                    let first = depths[0];
                    for d in &depths {
                        assert_eq!(*d, first, "leaves at unequal depths");
                    }
                }
            }
            // Child-mirror consistency: each ChildRef mirrors its child's totals.
            idx.walk_all_nodes(&host, &mut |node: &Node<String, u64>| {
                for c in node.children() {
                    let child = idx.load_node(&host, c.id);
                    assert_eq!(c.subtree_count.0, child.subtree_count());
                    assert_eq!(c.entry_count.0, child.slot_count() as u32);
                }
            });
            // STRENGTHENED B+ invariant: the leaf next-link chain, started from
            // the leftmost leaf, visits every leaf exactly once, is globally
            // key-sorted, and totally covers the index (sum of leaf entries ==
            // len). This pins the new structural guarantee the optimization
            // relies on (range walks the chain without re-descending).
            assert_leaf_chain_sorted_and_total(&idx, &host);
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

    fn walk_depths_node(
        idx: &OrderedIndex<String, u64, 2>,
        host: &Host,
        node: &Node<String, u64>,
        depth: usize,
        depths: &mut Vec<usize>,
    ) {
        if depth > 0 {
            assert!(
                node.slot_count() >= 1,
                "non-root node violates min-degree: 0 slots (need >= T-1 = 1)",
            );
        }
        match node {
            Node::Leaf { .. } => depths.push(depth),
            Node::Internal { children, .. } => {
                for c in children {
                    let child = idx.load_node(host, c.id);
                    walk_depths_node(idx, host, &child, depth + 1, depths);
                }
            }
        }
    }

    /// STRENGTHENED B+ invariant helper: walk the leaf next-link chain from
    /// the leftmost leaf and assert it is sorted, visits each leaf once, and
    /// covers every entry in the index.
    fn assert_leaf_chain_sorted_and_total(idx: &OrderedIndex<String, u64, 2>, host: &Host) {
        let Some(root) = idx.load_root(host) else {
            return;
        };
        // Descend to the leftmost leaf.
        let mut node = root;
        while let Node::Internal { children, .. } = &node {
            let child_id = children[0].id;
            node = idx.load_node(host, child_id);
        }
        let mut prev: Option<(String, u64)> = None;
        let mut total: u64 = 0;
        let mut seen = 0usize;
        loop {
            let Node::Leaf { entries, next } = &node else {
                panic!("leaf chain hit a non-leaf");
            };
            seen += 1;
            assert!(seen < 1_000_000, "leaf chain appears to loop");
            for e in entries {
                let cur = (e.key.clone(), e.nonce);
                if let Some(p) = &prev {
                    assert!(
                        p.0 < cur.0 || (p.0 == cur.0 && p.1 < cur.1.0),
                        "leaf chain not globally sorted",
                    );
                }
                prev = Some((cur.0, cur.1.0));
                total += 1;
            }
            match next {
                Some(n) => {
                    let n = *n;
                    node = idx.load_node(host, n);
                }
                None => break,
            }
        }
        assert_eq!(
            total,
            idx.len(host),
            "leaf chain does not cover all entries"
        );
    }
}
