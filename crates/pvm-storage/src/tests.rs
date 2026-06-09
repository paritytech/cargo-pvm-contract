extern crate alloc;
extern crate std;

use super::*;
use alloc::rc::Rc;
#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use pvm_contract_types::Address;
#[cfg(feature = "alloc")]
use pvm_contract_types::Bytes;
use pvm_contract_types::MockHostBuilder;
use ruint::aliases::U256;

/// Fresh isolated `Host` backed by a new `MockHost` in an `Rc`.
/// Clone the returned handle to share storage state between cells.
fn h() -> Host {
    Host::from_dyn(Rc::new(MockHostBuilder::new().build()))
}

// --- Lazy roundtrips ---

#[test]
fn lazy_roundtrip_u256() {
    let mut lazy = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&U256::from(42));
    assert_eq!(lazy.get(), U256::from(42));
}

#[test]
fn lazy_roundtrip_address() {
    let addr = Address([0xAA; 20]);
    let mut lazy = unsafe { Lazy::<Address>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&addr);
    assert_eq!(lazy.get(), addr);
}

#[test]
fn lazy_roundtrip_bool() {
    let mut lazy = unsafe { Lazy::<bool>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&true);
    assert!(lazy.get());
    lazy.set(&false);
    // Writing false = all-zero = deletes the key, so get returns zero = false
    assert!(!lazy.get());
}

#[test]
fn lazy_default_is_zero() {
    let lazy = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, h()) };
    assert_eq!(lazy.get(), U256::ZERO);
}

#[test]
fn lazy_try_get_uninitialized() {
    let lazy = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, h()) };
    assert_eq!(lazy.try_get(), None);
}

#[test]
fn lazy_try_get_nonzero_value() {
    let mut lazy = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&U256::from(99));
    assert_eq!(lazy.try_get(), Some(U256::from(99)));
}

#[test]
fn lazy_set_zero_deletes() {
    let mut lazy = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&U256::from(42));
    assert_eq!(lazy.try_get(), Some(U256::from(42)));
    lazy.set(&U256::ZERO);
    // Writing zero triggers set_storage_or_clear deletion
    assert_eq!(lazy.try_get(), None);
}

#[test]
fn lazy_clear_then_try_get() {
    let mut lazy = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&U256::from(42));
    lazy.clear();
    assert_eq!(lazy.try_get(), None);
}

#[test]
fn lazy_clear() {
    let mut lazy = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&U256::from(42));
    lazy.clear();
    assert_eq!(lazy.get(), U256::ZERO);
}

// --- Multi-slot Lazy<T> (T spans >1 storage slot) ---

#[test]
fn lazy_roundtrip_tuple_two_u256() {
    let mut lazy = unsafe { Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), 0, h()) };
    let v = (U256::from(7u64), U256::from(11u64));
    lazy.set(&v);
    assert_eq!(lazy.get(), v);
}

#[test]
fn lazy_multi_slot_writes_consecutive_keys() {
    // (U256, U256) has ENCODED_SIZE == 64, so set() must touch slots
    // `key` and `key + 1`. Confirm the wire format by reading the slots
    // directly: the first U256 lands at `key`, the second at `key + 1`.
    let mut lazy = unsafe { Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let base = *lazy.key.as_bytes();

    lazy.set(&(U256::from(0xAAu64), U256::from(0xBBu64)));

    let slot0 = storage_get_32(&host, &base);
    let mut next = base;
    inc_slot(&mut next);
    let slot1 = storage_get_32(&host, &next);

    assert_eq!(slot0[31], 0xAA, "first U256 at base slot: {slot0:?}");
    assert_eq!(slot1[31], 0xBB, "second U256 at base + 1: {slot1:?}");
}

#[test]
fn lazy_multi_slot_try_get_some_when_only_second_word_set() {
    // Direct-write a value where the first 32-byte word is zero but the
    // second is non-zero. try_get must still observe the entry as present.
    let host = h();
    let key = StorageKey::from_slot(0);
    let mut second = [0u8; 32];
    second[31] = 0x42;
    let mut next = *key.as_bytes();
    inc_slot(&mut next);
    storage_set_32(&host, &next, &second);

    let lazy = unsafe { Lazy::<(U256, U256)>::new(key, 0, host) };
    assert_eq!(lazy.try_get(), Some((U256::ZERO, U256::from(0x42u64))));
}

#[test]
fn lazy_multi_slot_try_get_none_when_unwritten() {
    let lazy = unsafe { Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), 0, h()) };
    assert_eq!(lazy.try_get(), None);
}

#[test]
fn lazy_multi_slot_clear_removes_all_words() {
    // Set both words non-zero, clear, then verify each underlying slot
    // is truly absent (not just zero in the decoded value).
    let mut lazy = unsafe { Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let base = *lazy.key.as_bytes();

    lazy.set(&(U256::from(1u64), U256::from(2u64)));
    lazy.clear();

    let mut next = base;
    assert_eq!(storage_try_get_32(&host, &next), None, "word 0 not cleared");
    inc_slot(&mut next);
    assert_eq!(storage_try_get_32(&host, &next), None, "word 1 not cleared");
}

#[test]
fn lazy_multi_slot_overwrite_zero_clears_stale_slot() {
    // After writing (5, 5), writing (5, 0) must auto-delete slot 1 so
    // try_get observes the zero on subsequent reads.
    let mut lazy = unsafe { Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let mut next = *lazy.key.as_bytes();
    inc_slot(&mut next);

    lazy.set(&(U256::from(5u64), U256::from(5u64)));
    lazy.set(&(U256::from(5u64), U256::ZERO));

    assert_eq!(lazy.get(), (U256::from(5u64), U256::ZERO));
    assert_eq!(
        storage_try_get_32(&host, &next),
        None,
        "stale slot for word 1 must be auto-deleted"
    );
}

#[test]
fn lazy_multi_slot_slots_const_matches_word_count() {
    // SLOTS = ENCODED_SIZE / 32. For (U256, U256) that's 2, so an
    // auto-numbered field after this Lazy would be 2 slots later.
    assert_eq!(<Lazy<U256> as StorageComponent>::SLOTS, 1);
    assert_eq!(<Lazy<(U256, U256)> as StorageComponent>::SLOTS, 2);
    assert_eq!(<Lazy<(U256, U256, U256)> as StorageComponent>::SLOTS, 3);
}

// --- Mapping operations ---

#[test]
fn mapping_insert_get() {
    let mut m = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(0), h()) };
    let addr = Address([0xBB; 20]);
    m.insert(&addr, &U256::from(100));
    assert_eq!(m.get(&addr), U256::from(100));
}

#[test]
fn mapping_remove() {
    let mut m = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(0), h()) };
    let addr = Address([0xCC; 20]);
    m.insert(&addr, &U256::from(50));
    m.remove(&addr);
    assert_eq!(m.get(&addr), U256::ZERO);
}

#[test]
fn mapping_remove_then_try_get() {
    let mut m = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(0), h()) };
    let addr = Address([0xDD; 20]);
    m.insert(&addr, &U256::from(50));
    assert_eq!(m.try_get(&addr), Some(U256::from(50)));
    m.remove(&addr);
    // Key is truly deleted, not just zeroed (#33)
    assert_eq!(m.try_get(&addr), None);
}

#[test]
fn mapping_different_keys_independent() {
    let mut m = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(0), h()) };
    let a = Address([0x01; 20]);
    let b = Address([0x02; 20]);
    m.insert(&a, &U256::from(10));
    m.insert(&b, &U256::from(20));
    assert_eq!(m.get(&a), U256::from(10));
    assert_eq!(m.get(&b), U256::from(20));
}

// --- Multi-slot Mapping<K, V> (V spans >1 storage slot) ---

#[test]
fn mapping_insert_get_tuple_value() {
    let mut m = unsafe { Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h()) };
    let addr = Address([0xAB; 20]);
    let v = (U256::from(123u64), U256::from(456u64));
    m.insert(&addr, &v);
    assert_eq!(m.get(&addr), v);
}

#[test]
fn mapping_multi_slot_remove_clears_all_words() {
    let mut m = unsafe { Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h()) };
    let host = m.host.clone();
    let addr = Address([0xCD; 20]);
    let derived = *m.slot_of(&addr).as_bytes();

    m.insert(&addr, &(U256::from(1u64), U256::from(2u64)));
    m.remove(&addr);

    let mut k = derived;
    assert_eq!(storage_try_get_32(&host, &k), None, "word 0 not removed");
    inc_slot(&mut k);
    assert_eq!(storage_try_get_32(&host, &k), None, "word 1 not removed");
    assert_eq!(m.try_get(&addr), None);
}

#[test]
fn mapping_multi_slot_overwrite_smaller_clears_stale_word() {
    // insert (1, 2) then insert (1, 0): the second word must be deleted
    // so a follow-up read doesn't return stale 2.
    let mut m = unsafe { Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h()) };
    let host = m.host.clone();
    let addr = Address([0xEF; 20]);
    let mut next = *m.slot_of(&addr).as_bytes();
    inc_slot(&mut next);

    m.insert(&addr, &(U256::from(1u64), U256::from(2u64)));
    m.insert(&addr, &(U256::from(1u64), U256::ZERO));

    assert_eq!(m.get(&addr), (U256::from(1u64), U256::ZERO));
    assert_eq!(storage_try_get_32(&host, &next), None);
}

#[test]
fn mapping_multi_slot_entry_handle_reads_and_writes_full_value() {
    // entry() returns a Lazy<V> at the derived key. With multi-slot V it
    // must still read/write all chunks correctly.
    let mut m = unsafe { Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h()) };
    let addr = Address([0x10; 20]);
    let v = (U256::from(99u64), U256::from(100u64));

    let mut cell = m.entry(&addr);
    cell.set(&v);
    assert_eq!(cell.get(), v);

    // And the parent Mapping reads back the same value through its own
    // derived key, confirming entry() didn't drift off the right key.
    assert_eq!(m.get(&addr), v);
}

// --- Nested mappings ---

#[test]
fn nested_mapping_allowances() {
    let mut allowances =
        unsafe { Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), h()) };
    let owner = Address([0xAA; 20]);
    let spender = Address([0xBB; 20]);

    allowances.entry(&owner).insert(&spender, &U256::from(500));
    assert_eq!(allowances.get(&owner).get(&spender), U256::from(500));
}

// --- Tuple keys ---

#[test]
fn tuple_key_matches_chaining() {
    let host = h();
    let owner = Address([0xAA; 20]);
    let spender = Address([0xBB; 20]);
    let amount = U256::from(123);

    // Write via nested mapping chaining
    let mut chained = unsafe {
        Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), host.clone())
    };
    chained.entry(&owner).insert(&spender, &amount);

    // Read via tuple key (same slot, same host state)
    let tuple_map =
        unsafe { Mapping::<(Address, Address), U256>::new(StorageKey::from_slot(2), host.clone()) };
    assert_eq!(tuple_map.get(&(owner, spender)), amount);
}

#[test]
fn tuple_key_write_and_read() {
    let mut m = unsafe { Mapping::<(Address, Address), U256>::new(StorageKey::from_slot(0), h()) };
    let alice = Address([0xAA; 20]);
    let bob = Address([0xBB; 20]);

    m.insert(&(alice, bob), &U256::from(500));
    assert_eq!(m.get(&(alice, bob)), U256::from(500));
    assert_eq!(m.get(&(bob, alice)), U256::ZERO); // different key order
}

#[test]
fn triple_tuple_key_matches_chaining() {
    let host = h();
    let a = Address([0xAA; 20]);
    let b = Address([0xBB; 20]);
    let c = Address([0xCC; 20]);

    // Derive slot via triple nesting
    let root = StorageKey::from_slot(0);
    let chained = root.derive(&host, &a);
    let chained = chained.derive(&host, &b);
    let chained = chained.derive(&host, &c);

    // Derive slot via 3-tuple (must match chaining)
    let tupled = (a, b, c).derive_slot(&host, &root);
    assert_eq!(chained, tupled);
}

#[test]
fn bytes32_as_mapping_key() {
    let mut m = unsafe { Mapping::<[u8; 32], U256>::new(StorageKey::from_slot(0), h()) };
    let key = [0xAB; 32];
    m.insert(&key, &U256::from(42));
    assert_eq!(m.get(&key), U256::from(42));
}

// --- Dynamic accessors: Lazy<String> / Lazy<Bytes> ---

#[cfg(feature = "alloc")]
#[test]
fn lazy_roundtrip_string_short() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&String::from("hello"));
    assert_eq!(lazy.get(), "hello");
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_roundtrip_string_long() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let long = "a".repeat(200);
    lazy.set(&long);
    assert_eq!(lazy.get(), long);
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_empty_is_default() {
    let lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    assert_eq!(lazy.get(), "");
    assert_eq!(lazy.try_get(), None);
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_clear() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&String::from("payload"));
    assert_eq!(lazy.try_get().as_deref(), Some("payload"));
    lazy.clear();
    assert_eq!(lazy.try_get(), None);
    assert_eq!(lazy.get(), "");
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_overwrite_smaller() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    let long =
        String::from("hello world this is a long string that spills over the inline boundary");
    let long_chunks = long.len().div_ceil(32);
    lazy.set(&long);
    lazy.set(&String::from("short"));
    assert_eq!(lazy.get(), "short");

    // Stale body chunks from the previous long value must have been
    // deleted, otherwise we'd be leaking storage on every long → short
    // transition.
    let mut body_slot = dynamic_data_root(&host, key.as_bytes());
    for _ in 0..long_chunks {
        assert_eq!(
            storage_try_get_32(&host, &body_slot),
            None,
            "stale body chunk not cleared"
        );
        inc_slot(&mut body_slot);
    }
}

// --- solc layout invariants ---

/// "set("") and never written are distinguishable" — the central guarantee
/// of using raw set_storage (not _or_clear) for the short header.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_set_empty_distinct_from_never_written() {
    let mut written = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let never = unsafe { Lazy::<String>::new(StorageKey::from_slot(1), 0, written.host.clone()) };

    written.set(&String::new());

    assert_eq!(written.try_get(), Some(String::new()));
    assert_eq!(written.get(), "");
    assert_eq!(never.try_get(), None);
    assert_eq!(never.get(), "");
}

/// `set("")` must leave a non-zero header in the slot so that
/// `set_storage_or_clear` doesn't auto-delete it; the decoder still
/// reports inline-len-0. The sentinel lives at `slot[30]` (outside the
/// zero-length body and outside the length byte at `slot[31]`).
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_set_empty_writes_non_zero_sentinel_header() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;

    lazy.set(&String::new());

    let slot_bytes = storage_get_32(&host, key.as_bytes());
    assert_ne!(
        slot_bytes, [0u8; 32],
        "slot must be non-zero so it persists"
    );
    assert_eq!(slot_bytes[31], 0, "length byte: inline + len 0");
    assert_eq!(slot_bytes[30], EMPTY_INLINE_SENTINEL, "sentinel at byte 30");
    assert!(
        slot_bytes[..30].iter().all(|&b| b == 0),
        "bytes 0..30 must be zero"
    );
}

/// Overwriting a sentinel-only empty header with a non-empty value must
/// clear the sentinel byte (otherwise stale `0x01` at `slot[30]` would
/// land inside a future 31-byte inline value's body).
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_overwrite_empty_clears_sentinel() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;

    lazy.set(&String::new());
    lazy.set(&"a".repeat(31));

    let slot_bytes = storage_get_32(&host, key.as_bytes());
    assert_eq!(
        slot_bytes[30], b'a',
        "byte 30 is the last body byte for len=31"
    );
    assert_eq!(slot_bytes[31], 31 * 2, "length × 2");
    assert_eq!(lazy.get(), "a".repeat(31));
}

/// Probe the slot bytes directly: short value lives inline with
/// `byte31 = length * 2` (low bit = 0). This is the solc convention that
/// `cast storage` decodes natively.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_short_inline_layout() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    lazy.set(&String::from("hello"));

    let slot_bytes = storage_get_32(&host, key.as_bytes());
    assert_eq!(&slot_bytes[..5], b"hello");
    assert!(slot_bytes[5..31].iter().all(|&b| b == 0));
    assert_eq!(slot_bytes[31], 5 * 2, "byte31 = length * 2, low bit 0");
}

/// 31-byte string is still inline; 32-byte string spills.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_boundary_31_bytes_inline() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    let s = "a".repeat(31);
    lazy.set(&s);

    let slot_bytes = storage_get_32(&host, key.as_bytes());
    assert!(slot_bytes[31] & 1 == 0, "low bit 0 -> inline");
    assert_eq!(slot_bytes[31] >> 1, 31);
    assert_eq!(&slot_bytes[..31], s.as_bytes());
    assert_eq!(lazy.get(), s);
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_boundary_32_bytes_spilled() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    let s = "b".repeat(32);
    lazy.set(&s);

    let slot_bytes = storage_get_32(&host, key.as_bytes());
    assert!(slot_bytes[31] & 1 == 1, "low bit 1 -> spilled");
    // Header = 32 * 2 + 1 = 65, fits in byte 31.
    assert_eq!(slot_bytes[31], 65);
    assert!(slot_bytes[..31].iter().all(|&b| b == 0));
    assert_eq!(lazy.get(), s);
}

/// A spilled header (low bit of byte 31 set) with non-zero bytes in the
/// upper half of the u256 length field cannot be a real stored length —
/// any plausible value fits in the low 128 bits. Without validation the
/// decoder would silently use the truncated low bits and `read_dyn_body`
/// would walk a fabricated number of chunks. The decoder now treats any
/// such slot as empty.
#[cfg(feature = "alloc")]
#[test]
fn lazy_bytes_spilled_high_bytes_treated_as_malformed() {
    let host = h();
    let key = StorageKey::from_slot(0);
    let mut malformed = [0u8; 32];
    malformed[0] = 0xFF; // non-zero high byte ⇒ malformed
    malformed[31] = 0x01; // low bit set ⇒ spilled
    storage_set_32(&host, key.as_bytes(), &malformed);

    let lazy = unsafe { Lazy::<Bytes>::new(key, 0, host) };
    assert!(lazy.get().0.is_empty());
}

/// A malformed inline header (byte31 > 62, low bit 0) encodes a decoded
/// length > 31. Without a cap, `dynamic_bytes_get` would slice past the
/// 32-byte slot buffer and panic. The decoder caps `len` at 31 so reads
/// of corrupted / foreign-written slots return at most 31 bytes instead.
#[cfg(feature = "alloc")]
#[test]
fn lazy_bytes_inline_len_capped_on_malformed_slot() {
    let host = h();
    let key = StorageKey::from_slot(0);
    // byte31 = 0xFE → decoded len = 127 (way past slot capacity).
    let mut malformed = [0u8; 32];
    for (i, b) in malformed.iter_mut().enumerate().take(31) {
        *b = i as u8 + 1;
    }
    malformed[31] = 0xFE;
    storage_set_32(&host, key.as_bytes(), &malformed);

    let lazy = unsafe { Lazy::<Bytes>::new(key, 0, host) };
    // Must not panic. Cap is 31 bytes — the original 31 prefix bytes.
    let bytes = lazy.get();
    assert_eq!(bytes.0.len(), 31);
    assert_eq!(&bytes.0[..], &malformed[..31]);
}

/// Long-spill probe: header is `len * 2 + 1` big-endian, body chunks live
/// at consecutive slots starting from `keccak256(slot)`.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_long_spill_layout() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    // 40 bytes spans two 32-byte chunks (8 bytes into the second).
    let s: String = (0..40).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    lazy.set(&s);

    let slot_bytes = storage_get_32(&host, key.as_bytes());
    assert!(slot_bytes[31] & 1 == 1);
    // 40 * 2 + 1 = 81.
    assert_eq!(slot_bytes[31], 81);

    let mut body_slot = dynamic_data_root(&host, key.as_bytes());
    let chunk0 = storage_get_32(&host, &body_slot);
    assert_eq!(&chunk0[..32], &s.as_bytes()[..32]);

    inc_slot(&mut body_slot);
    let chunk1 = storage_get_32(&host, &body_slot);
    assert_eq!(&chunk1[..8], &s.as_bytes()[32..40]);
    assert!(chunk1[8..].iter().all(|&b| b == 0), "trailing chunk pad");

    assert_eq!(lazy.get(), s);
}

/// Short → long transition: previously inline data is replaced with
/// spill-form header and body chunks.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_grow_short_to_long() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&String::from("short"));
    assert_eq!(lazy.get(), "short");

    let long = "x".repeat(100);
    lazy.set(&long);
    assert_eq!(lazy.get(), long);
}

/// Long → short transition deletes the now-orphaned body chunks. Probes
/// each previously-occupied keccak slot and asserts it no longer exists.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_shrink_long_to_short_clears_chunks() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    let long = "y".repeat(100); // 4 chunks of 32B
    lazy.set(&long);
    lazy.set(&String::from("ok"));
    assert_eq!(lazy.get(), "ok");

    let mut body_slot = dynamic_data_root(&host, key.as_bytes());
    for chunk_idx in 0..4 {
        assert_eq!(
            storage_try_get_32(&host, &body_slot),
            None,
            "body chunk {chunk_idx} not cleared after shrink"
        );
        inc_slot(&mut body_slot);
    }
}

/// clear() on a long value must delete header AND every body chunk.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_clear_after_long_deletes_chunks() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    let long = "z".repeat(70); // 3 chunks
    lazy.set(&long);
    lazy.clear();

    // Header slot gone.
    assert_eq!(storage_try_get_32(&host, key.as_bytes()), None);
    // All body chunks gone.
    let mut body_slot = dynamic_data_root(&host, key.as_bytes());
    for chunk_idx in 0..3 {
        assert_eq!(
            storage_try_get_32(&host, &body_slot),
            None,
            "body chunk {chunk_idx} survived clear()"
        );
        inc_slot(&mut body_slot);
    }
    assert_eq!(lazy.try_get(), None);
    assert_eq!(lazy.get(), "");
}

/// `Mapping<Address, String>` with a spill-form value round-trips through
/// the same layout path.
#[cfg(feature = "alloc")]
#[test]
fn mapping_with_long_string_value() {
    let mut m = unsafe { Mapping::<Address, String>::new(StorageKey::from_slot(0), h()) };
    let addr = Address([0x11; 20]);
    let value = "w".repeat(100);
    m.insert(&addr, &value);
    assert_eq!(m.get(&addr), value);
    m.remove(&addr);
    assert_eq!(m.try_get(&addr), None);
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_roundtrip_bytes() {
    let mut lazy = unsafe { Lazy::<Bytes>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&Bytes(alloc::vec![1, 2, 3, 4, 5]));
    assert_eq!(lazy.get(), Bytes(alloc::vec![1, 2, 3, 4, 5]));
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_bytes_large() {
    let mut lazy = unsafe { Lazy::<Bytes>::new(StorageKey::from_slot(0), 0, h()) };
    let data = Bytes((0..=255u8).collect());
    lazy.set(&data);
    assert_eq!(lazy.get(), data);
}

/// `Bytes` rides the same solc-compatible path as `String`. Cover the
/// inline / spill boundary explicitly: 31 bytes inline, 32 bytes spills.
#[cfg(feature = "alloc")]
#[test]
fn lazy_bytes_boundary() {
    let mut a = unsafe { Lazy::<Bytes>::new(StorageKey::from_slot(0), 0, h()) };
    let host = a.host.clone();
    let key_a = a.key;

    let inline = Bytes((0..31).collect());
    a.set(&inline);
    let slot_bytes = storage_get_32(&host, key_a.as_bytes());
    assert_eq!(slot_bytes[31], 31 * 2, "31B vec inline, byte31 = 62");
    assert_eq!(a.get(), inline);

    let mut b = unsafe { Lazy::<Bytes>::new(StorageKey::from_slot(1), 0, host) };
    let spill = Bytes((0..32).collect());
    b.set(&spill);
    let slot_b = storage_get_32(&b.host, b.key.as_bytes());
    assert_eq!(slot_b[31], 32 * 2 + 1, "32B vec spills, byte31 = 65");
    assert_eq!(b.get(), spill);
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_address_to_string() {
    let mut m = unsafe { Mapping::<Address, String>::new(StorageKey::from_slot(0), h()) };
    let a = Address([0x01; 20]);
    let b = Address([0x02; 20]);
    m.insert(&a, &String::from("alice"));
    m.insert(&b, &String::from("bob"));
    assert_eq!(m.get(&a), "alice");
    assert_eq!(m.get(&b), "bob");
    m.remove(&a);
    assert_eq!(m.try_get(&a), None);
    assert_eq!(m.get(&b), "bob");
}

#[cfg(feature = "alloc")]
#[test]
fn dynamic_data_root_independent_per_slot() {
    // Distinct header slots must hash to distinct data roots so two
    // dynamic values on adjacent slots can't trample each other.
    let mut a = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = a.host.clone();
    let mut b = unsafe { Lazy::<String>::new(StorageKey::from_slot(1), 0, host) };
    a.set(&String::from("first"));
    b.set(&String::from("second"));
    assert_eq!(a.get(), "first");
    assert_eq!(b.get(), "second");
}

// --- Solidity compatibility ---

#[test]
fn storage_key_from_slot() {
    assert_eq!(StorageKey::from_slot(0).as_bytes(), &[0u8; 32]);
    let mut expected = [0u8; 32];
    expected[31] = 1;
    assert_eq!(StorageKey::from_slot(1).as_bytes(), &expected);
}

#[test]
fn derive_key_matches_solidity() {
    let host = h();
    // cast index address 0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA 1
    // Expected: keccak256(pad32(0xAA..AA) ++ pad32(1))
    let addr = Address([0xAA; 20]);
    let root = StorageKey::from_slot(1);
    let derived = root.derive(&host, &addr);

    // Compute expected: keccak256(0x000..0xAAAA..AA ++ 0x000..001)
    let mut preimage = [0u8; 64];
    // Address is right-aligned: 12 zero bytes + 20 address bytes
    preimage[12..32].copy_from_slice(&[0xAA; 20]);
    // Slot 1: 31 zero bytes + 0x01
    preimage[63] = 1;
    let mut expected = [0u8; 32];
    host.hash_keccak_256(&preimage, &mut expected);

    assert_eq!(derived.as_bytes(), &expected);
}

// --- StorageComponent ---

#[test]
fn storage_component_slot_count() {
    assert_eq!(<Lazy<U256> as StorageComponent>::SLOTS, 1);
    assert_eq!(<Mapping<Address, U256> as StorageComponent>::SLOTS, 1);
}

#[cfg(feature = "alloc")]
#[test]
fn storage_component_slot_count_dynamic() {
    assert_eq!(<Lazy<String> as StorageComponent>::SLOTS, 1);
    assert_eq!(<Lazy<Bytes> as StorageComponent>::SLOTS, 1);
    assert_eq!(<Mapping<Address, String> as StorageComponent>::SLOTS, 1);
    assert_eq!(<Mapping<Address, Bytes> as StorageComponent>::SLOTS, 1);
}

// --- Packing semantics (matches solc storageLayout) ---

/// Adjacent contract storage fields of `Lazy<u128>` pack into a single
/// 32-byte slot via the macro's `layout_step` walker — byte-identical to
/// solc's layout for `contract C { uint128 a; uint128 b; }` (a at
/// offset 16, b at offset 0).
///
/// Verifies the `StorageComponent::PACKED_BYTES` propagation and the
/// const-folded walker's placement directly.
#[test]
fn adjacent_lazy_u128_packs_at_contract_field_level() {
    assert_eq!(<u128 as StorageEncode>::PACKED_BYTES, 16);
    assert_eq!(<u128 as StorageEncode>::STORAGE_SLOTS, 1);
    assert_eq!(<Lazy<u128> as StorageComponent>::SLOTS, 1);
    assert_eq!(<Lazy<u128> as StorageComponent>::PACKED_BYTES, 16);

    // Two-step walker walk: first u128 at (slot=0, offset=16);
    // second u128 at (slot=0, offset=0).
    let step_a = crate::layout_step(crate::LayoutStep::FIRST, 16, 1);
    let step_b = crate::layout_step(step_a, 16, 1);
    assert_eq!(step_a.slot, 0);
    assert_eq!(step_a.offset, 16);
    assert_eq!(step_b.slot, 0);
    assert_eq!(step_b.offset, 0);
    assert_eq!(step_b.next_slot, 0);
    assert_eq!(step_b.next_space, 0);
}

/// Wire-level packing: two `Lazy<u128>` fields placed by the layout
/// walker share slot 0 — `a` at offset 16, `b` at offset 0 — matching
/// solc's `uint128 a; uint128 b;` storage layout exactly.
#[test]
fn two_lazy_u128_cells_pack_into_one_slot() {
    let host = h();
    // Walker placement: first u128 at (0, 16), second at (0, 0).
    let mut a = unsafe { Lazy::<u128>::new(StorageKey::from_slot(0), 16, host.clone()) };
    let mut b = unsafe { Lazy::<u128>::new(StorageKey::from_slot(0), 0, host.clone()) };

    a.set(&0x1111_1111_1111_1111u128);
    b.set(&0x2222_2222_2222_2222u128);

    let slot_0 = storage_get_32(&host, &StorageKey::from_slot(0).as_bytes().clone());
    let slot_1 = storage_get_32(&host, &StorageKey::from_slot(1).as_bytes().clone());

    // a lives at bytes 16..32 (right-aligned u128); b at bytes 0..16.
    assert_eq!(
        &slot_0[16..32],
        &0x1111_1111_1111_1111u128.to_be_bytes(),
        "slot 0 bytes 16..31 hold `a`",
    );
    assert_eq!(
        &slot_0[0..16],
        &0x2222_2222_2222_2222u128.to_be_bytes(),
        "slot 0 bytes 0..15 hold `b` — packing matches solc",
    );
    // Slot 1 stays empty: only one storage slot consumed for both fields.
    assert_eq!(slot_1, [0u8; 32], "slot 1 untouched — packing saved a slot");

    // Round-trip reads through both handles.
    assert_eq!(a.get(), 0x1111_1111_1111_1111u128);
    assert_eq!(b.get(), 0x2222_2222_2222_2222u128);
}

/// Classic solc packing example:
/// `contract C { bool a; uint32 b; address c; uint256 d; }` lays out as
/// slot 0: a (offset 31, 1 byte), b (offset 27, 4 bytes), c (offset 7, 20 bytes)
/// slot 1: d (offset 0, 32 bytes).
/// The const-folded walker should reproduce these placements byte-for-byte.
#[test]
fn classic_solc_layout_packs_bool_u32_address_into_one_slot() {
    let step_a = crate::layout_step(crate::LayoutStep::FIRST, 1, 1);
    let step_b = crate::layout_step(step_a, 4, 1);
    let step_c = crate::layout_step(step_b, 20, 1);
    let step_d = crate::layout_step(step_c, 32, 1);

    assert_eq!(
        (step_a.slot, step_a.offset),
        (0, 31),
        "bool at slot 0 offset 31"
    );
    assert_eq!(
        (step_b.slot, step_b.offset),
        (0, 27),
        "u32 at slot 0 offset 27"
    );
    assert_eq!(
        (step_c.slot, step_c.offset),
        (0, 7),
        "address at slot 0 offset 7"
    );
    assert_eq!(
        (step_d.slot, step_d.offset),
        (1, 0),
        "U256 at slot 1 offset 0"
    );
}

/// RMW correctness: writing one packed field does not clobber the other
/// occupying the same slot. Repeat with reversed write order to confirm
/// neither direction loses data.
#[test]
fn packed_u128_rmw_preserves_neighbour_both_directions() {
    for (write_a_first, label) in [(true, "a then b"), (false, "b then a")] {
        let host = h();
        let mut a = unsafe { Lazy::<u128>::new(StorageKey::from_slot(0), 16, host.clone()) };
        let mut b = unsafe { Lazy::<u128>::new(StorageKey::from_slot(0), 0, host.clone()) };

        let av = 0xAAAA_AAAA_AAAA_AAAAu128;
        let bv = 0xBBBB_BBBB_BBBB_BBBBu128;
        if write_a_first {
            a.set(&av);
            b.set(&bv);
        } else {
            b.set(&bv);
            a.set(&av);
        }
        assert_eq!(a.get(), av, "{label}: a survived");
        assert_eq!(b.get(), bv, "{label}: b survived");
    }
}

/// Clear-preserves-neighbours: clearing one packed field zeroes only its
/// byte window. The slot stays non-zero because the other field is still
/// written, so `set_storage_or_clear` does not auto-delete the slot.
#[test]
fn packed_u128_clear_preserves_neighbour() {
    let host = h();
    let mut a = unsafe { Lazy::<u128>::new(StorageKey::from_slot(0), 16, host.clone()) };
    let mut b = unsafe { Lazy::<u128>::new(StorageKey::from_slot(0), 0, host.clone()) };

    a.set(&0xAAAA_AAAA_AAAA_AAAAu128);
    b.set(&0xBBBB_BBBB_BBBB_BBBBu128);
    b.clear();

    assert_eq!(
        a.get(),
        0xAAAA_AAAA_AAAA_AAAAu128,
        "a untouched after b.clear()"
    );
    assert_eq!(b.get(), 0, "b is zero after clear");
    // Slot stays non-zero overall (a's bytes are still there).
    let slot = storage_get_32(&host, &StorageKey::from_slot(0).as_bytes().clone());
    assert_ne!(slot, [0u8; 32], "slot retained — a kept it alive");
}

/// Multi-slot composite (`(U256, U256)`) starts a fresh slot and consumes
/// it to the end, so the next field starts at a new slot.
#[test]
fn multi_slot_composite_forces_fresh_slot_for_next_field() {
    // bool + (U256, U256) + u32 layout.
    let step_bool = crate::layout_step(crate::LayoutStep::FIRST, 1, 1);
    let step_tuple = crate::layout_step(step_bool, 32, 2);
    let step_u32 = crate::layout_step(step_tuple, 4, 1);

    assert_eq!((step_bool.slot, step_bool.offset), (0, 31));
    assert_eq!(
        (step_tuple.slot, step_tuple.offset),
        (1, 0),
        "tuple starts fresh"
    );
    assert_eq!(step_tuple.next_slot, 2, "tuple consumes slots 1 and 2");
    assert_eq!(step_tuple.next_space, 0, "tuple consumed slot 2 to its end");
    assert_eq!(
        (step_u32.slot, step_u32.offset),
        (3, 28),
        "u32 lands at slot 3"
    );
}

/// Walker sanity check: `Mapping` reports `PACKED_BYTES = 32` so it
/// always advances to a fresh slot and never packs with neighbours.
#[test]
fn mapping_packed_bytes_is_full_slot() {
    assert_eq!(
        <Mapping<Address, U256> as StorageComponent>::PACKED_BYTES,
        32
    );
    // bool + mapping + bool: mapping forces fresh slot; second bool can
    // pack at offset 31 of its own fresh slot (post-mapping).
    let step_a = crate::layout_step(crate::LayoutStep::FIRST, 1, 1);
    let step_map = crate::layout_step(step_a, 32, 1);
    let step_b = crate::layout_step(step_map, 1, 1);

    assert_eq!((step_a.slot, step_a.offset), (0, 31));
    assert_eq!((step_map.slot, step_map.offset), (1, 0));
    assert_eq!((step_b.slot, step_b.offset), (2, 31));
}

// --- Solidity `string` decode is lossy ---

/// `Lazy<String>::get()` silently substitutes invalid UTF-8 with U+FFFD.
///
/// Rationale (already in `storage_codec::String::read_from_storage`):
/// a foreign contract sharing the same storage slot may have written
/// non-UTF-8 bytes. Trapping the read would brick our contract; lossy
/// decoding preserves liveness at the cost of silent data substitution.
///
/// User impact: contracts that *require* exact byte preservation must
/// use `Lazy<Bytes>` (or `Lazy<Vec<u8>>`), not `Lazy<String>`.
#[cfg(feature = "alloc")]
#[test]
fn lazy_string_decode_silently_replaces_invalid_utf8_with_replacement_char() {
    let host = h();
    let key = StorageKey::from_slot(0);

    // Short-form solidity `string` header: byte 31 = len * 2 (low bit 0 = inline).
    // Body bytes 0..3 are an isolated UTF-8 continuation (0xff is never
    // valid as a leading or continuation byte).
    let mut raw = [0u8; 32];
    raw[0] = 0xff;
    raw[1] = 0xfe;
    raw[2] = 0xfd;
    raw[3] = 0xfc;
    raw[31] = 4 * 2;
    storage_set_32(&host, key.as_bytes(), &raw);

    let lazy = unsafe { Lazy::<String>::new(key, 0, host.clone()) };
    let read = lazy.get();
    // Each invalid byte becomes one U+FFFD. The roundtrip is *not* the
    // bytes we wrote — this is the lossy substitution the docstring on
    // try_get does not currently mention.
    assert_eq!(
        read.chars().filter(|c| *c == '\u{FFFD}').count(),
        4,
        "all four invalid bytes substituted; no error returned"
    );

    // Counter-check: the same wire bytes through `Lazy<Bytes>` preserve
    // the exact content, no substitution.
    let lazy_bytes = unsafe { Lazy::<Bytes>::new(key, 0, host) };
    let preserved = lazy_bytes.get();
    assert_eq!(preserved.0, alloc::vec![0xff, 0xfe, 0xfd, 0xfc]);
}

#[test]
fn storage_component_new_at_matches_new() {
    let host = h();
    let mut a = unsafe { Lazy::<U256>::new(StorageKey::from_slot(7), 0, host.clone()) };
    let mut b = <Lazy<U256> as StorageComponent>::new_at(7, 0, host);
    a.set(&U256::from(99));
    // `b` shares the host, so should see the same write.
    assert_eq!(b.get(), U256::from(99));
    b.set(&U256::from(100));
    assert_eq!(a.get(), U256::from(100));
}

// --- Solidity zero-slot semantics ---

/// `insert(k, &V::default())` deletes the slot (matching `SSTORE`-clears),
/// so a subsequent `try_get` returns `None` even though we just wrote.
/// Pinned here so the conflation between "never written" and "explicit
/// zero" stays documented behavior, not an accidental regression.
#[test]
fn try_get_returns_none_after_inserting_zero() {
    let host = h();
    let mut m = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(0), host) };
    let addr = Address([0x77; 20]);

    m.insert(&addr, &U256::from(42));
    assert_eq!(m.try_get(&addr), Some(U256::from(42)));

    m.insert(&addr, &U256::ZERO);
    assert_eq!(m.try_get(&addr), None);
    assert_eq!(m.get(&addr), U256::ZERO);
}

// --- Entry optimization ---

#[test]
fn entry_reuse_for_read_write() {
    let mut m = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(0), h()) };
    let addr = Address([0xEE; 20]);
    m.insert(&addr, &U256::from(100));

    // Use entry for read-then-write
    let mut cell = m.entry(&addr);
    let val = cell.get();
    assert_eq!(val, U256::from(100));
    cell.set(&(val - U256::from(30)));

    assert_eq!(m.get(&addr), U256::from(70));
}

/// `Mapping::entry()` and `Mapping::insert()` must produce IDENTICAL byte
/// layouts in the derived slot for any sub-word `V` — both should match
/// solc's right-aligned placement at byte `32 - V::PACKED_BYTES`.
///
/// Regression test: an earlier `entry()` hardcoded `offset=0`, which
/// caused `entry().set(v)` to write at bytes 0..PACKED_BYTES while
/// `insert` / `get` operated at bytes `32-PACKED_BYTES..32`. Round-trips
/// through mixed paths silently lost data, and the storage layout
/// disagreed with `cast storage` on the same slot.
///
/// Covers `u128`, `bool`, and `Address` — three representative
/// sub-word primitives with distinct `PACKED_BYTES` (16, 1, 20).
#[test]
fn entry_set_matches_insert_for_subword_v() {
    // Run the same cross-check for u128.
    {
        let host = h();
        let m1_root = StorageKey::from_slot(0);
        let m2_root = StorageKey::from_slot(1);
        let mut m1 = unsafe { Mapping::<Address, u128>::new(m1_root, host.clone()) };
        let mut m2 = unsafe { Mapping::<Address, u128>::new(m2_root, host.clone()) };
        let key = Address([0xAA; 20]);
        let v: u128 = 0xCAFE_BABE_DEAD_BEEFu128;

        m1.entry(&key).set(&v);
        m2.insert(&key, &v);

        // Round-trips agree via either path on either map.
        assert_eq!(m1.get(&key), v, "u128: entry().set then m.get");
        assert_eq!(m2.get(&key), v, "u128: insert then m.get");
        assert_eq!(
            m1.entry(&key).get(),
            v,
            "u128: entry().set then entry().get"
        );
        assert_eq!(m2.entry(&key).get(), v, "u128: insert then entry().get");

        // Slot bytes are byte-identical; both place the u128 at solc's
        // canonical offset 16..32.
        let slot1 = storage_get_32(&host, m1.slot_of(&key).as_bytes());
        let slot2 = storage_get_32(&host, m2.slot_of(&key).as_bytes());
        assert_eq!(slot1, slot2, "u128: entry vs insert slot bytes");
        assert_eq!(
            &slot2[16..32],
            &v.to_be_bytes(),
            "u128: solc canonical placement is bytes 16..32",
        );
        assert!(
            slot2[..16].iter().all(|&b| b == 0),
            "u128: bytes 0..16 are zero (no neighbour in a Mapping entry)",
        );
    }

    // bool: PACKED_BYTES = 1, canonical offset = 31.
    {
        let host = h();
        let mut m1 = unsafe { Mapping::<u64, bool>::new(StorageKey::from_slot(0), host.clone()) };
        let mut m2 = unsafe { Mapping::<u64, bool>::new(StorageKey::from_slot(1), host.clone()) };
        let key: u64 = 7;

        m1.entry(&key).set(&true);
        m2.insert(&key, &true);

        assert!(m1.get(&key), "bool: entry().set then m.get");
        assert!(m1.entry(&key).get(), "bool: entry().set then entry().get");
        assert!(m2.entry(&key).get(), "bool: insert then entry().get");

        let slot1 = storage_get_32(&host, m1.slot_of(&key).as_bytes());
        let slot2 = storage_get_32(&host, m2.slot_of(&key).as_bytes());
        assert_eq!(slot1, slot2, "bool: entry vs insert slot bytes");
        assert_eq!(slot2[31], 1, "bool: solc canonical placement is byte 31");
        assert!(
            slot2[..31].iter().all(|&b| b == 0),
            "bool: bytes 0..31 are zero",
        );
    }

    // Address: PACKED_BYTES = 20, canonical offset = 12.
    {
        let host = h();
        let mut m1 =
            unsafe { Mapping::<u64, Address>::new(StorageKey::from_slot(0), host.clone()) };
        let mut m2 =
            unsafe { Mapping::<u64, Address>::new(StorageKey::from_slot(1), host.clone()) };
        let key: u64 = 42;
        let addr = Address([0x42; 20]);

        m1.entry(&key).set(&addr);
        m2.insert(&key, &addr);

        assert_eq!(m1.get(&key), addr, "Address: entry().set then m.get");
        assert_eq!(m2.get(&key), addr, "Address: insert then m.get");

        let slot1 = storage_get_32(&host, m1.slot_of(&key).as_bytes());
        let slot2 = storage_get_32(&host, m2.slot_of(&key).as_bytes());
        assert_eq!(slot1, slot2, "Address: entry vs insert slot bytes");
        assert_eq!(
            &slot2[12..32],
            &addr.0,
            "Address: solc canonical placement is bytes 12..32",
        );
    }
}

/// `entry().clear()` must zero the same byte window that `insert` wrote
/// — otherwise an `insert` followed by `entry().clear()` would leave the
/// value intact at solc's canonical offset.
#[test]
fn entry_clear_undoes_insert_for_subword_v() {
    let host = h();
    let mut m = unsafe { Mapping::<u64, u128>::new(StorageKey::from_slot(0), host.clone()) };
    let key: u64 = 1;
    m.insert(&key, &0xDEAD_BEEFu128);
    assert_eq!(m.get(&key), 0xDEAD_BEEFu128);

    m.entry(&key).clear();
    assert_eq!(m.get(&key), 0, "entry().clear must zero what insert wrote");
    // Slot should be auto-deleted (no neighbour exists in a Mapping entry).
    assert_eq!(
        storage_get_32(&host, m.slot_of(&key).as_bytes()),
        [0u8; 32],
        "all-zero packed write triggers host auto-delete",
    );
}

/// Same `entry().set()` vs `insert` parity check at the INNER level of a
/// nested `Mapping<K1, Mapping<K2, V>>` for sub-word `V`. The outer
/// `entry(k1)` returns a `RefMut<Mapping<K2, V>>`; the inner `entry(k2)`
/// inherits the same `Mapping::entry` path, so the offset bug would
/// propagate without this regression check.
#[test]
fn nested_mapping_entry_set_matches_insert_for_subword_v() {
    let host = h();
    let mut m1 = unsafe {
        Mapping::<Address, Mapping<Address, u128>>::new(StorageKey::from_slot(0), host.clone())
    };
    let mut m2 = unsafe {
        Mapping::<Address, Mapping<Address, u128>>::new(StorageKey::from_slot(1), host.clone())
    };
    let k1 = Address([0xAA; 20]);
    let k2 = Address([0xBB; 20]);
    let v: u128 = 0x1234_5678_90AB_CDEFu128;

    m1.entry(&k1).entry(&k2).set(&v);
    m2.entry(&k1).insert(&k2, &v);

    // Outer get → Ref<inner>, inner .get(k2) → V.
    assert_eq!(
        m1.get(&k1).get(&k2),
        v,
        "nested: entry-entry-set then get-get"
    );
    assert_eq!(m2.get(&k1).get(&k2), v, "nested: entry-insert then get-get");

    // Inspect the deepest derived slot via the inner mapping's slot_of
    // (which is reachable through Ref<Mapping<K2, V>>::slot_of since
    // slot_of takes `&self`).
    let inner_slot_1 = m1.get(&k1).slot_of(&k2);
    let inner_slot_2 = m2.get(&k1).slot_of(&k2);
    let slot1 = storage_get_32(&host, inner_slot_1.as_bytes());
    let slot2 = storage_get_32(&host, inner_slot_2.as_bytes());
    assert_eq!(slot1, slot2, "nested: entry vs insert produce same bytes");
    assert_eq!(
        &slot2[16..32],
        &v.to_be_bytes(),
        "nested: u128 at solc canonical bytes 16..32",
    );
}

// ---------------------------------------------------------------------
// Parametric packing-parity invariants
//
// Codifies the three invariants every packable primitive must obey across
// every container surface. If these hold for the full primitive set, the
// `Mapping::entry` offset-mismatch bug (and any future bug of the same
// shape) cannot exist.
//
// 1. Cross-write parity — `Lazy::new(slot, canonical).set`,
//    `Mapping::insert`, `Mapping::entry().set`, and
//    `StorageComponent::new_at(slot, canonical).set` all produce
//    byte-identical 32-byte slot contents.
// 2. Cross-read parity — every read surface returns the same value from
//    that slot.
// 3. Solc canonical placement — the value lives at bytes
//    `[32 - PACKED_BYTES .. 32]`, with the bytes above it zero in a
//    single-occupant slot.
// ---------------------------------------------------------------------

/// Run the three invariants for one `(V, sample_value, expected_tail)`
/// instantiation. `tail` must be the bytes solc places at
/// `slot[32 - PACKED_BYTES .. 32]` after a canonical write.
fn check_packing_parity<V>(name: &str, sample: V, tail: &[u8])
where
    V: StorageEncode + StorageDecode + Copy + PartialEq + core::fmt::Debug,
{
    let host = h();
    let canonical = (32 - V::PACKED_BYTES) as u8;

    // --- Four write paths into distinct slots ---
    let key_lazy_new = StorageKey::from_slot(0);
    let key_map_insert_root = StorageKey::from_slot(1);
    let key_map_entry_root = StorageKey::from_slot(2);
    let key_component = StorageKey::from_slot(3);

    // 1. Lazy::new + set
    {
        let mut lazy = unsafe { Lazy::<V>::new(key_lazy_new, canonical, host.clone()) };
        lazy.set(&sample);
    }
    // 2. Mapping::insert
    let mut m_insert = unsafe { Mapping::<u64, V>::new(key_map_insert_root, host.clone()) };
    m_insert.insert(&1u64, &sample);
    let map_insert_slot = m_insert.slot_of(&1u64);
    // 3. Mapping::entry().set
    let mut m_entry = unsafe { Mapping::<u64, V>::new(key_map_entry_root, host.clone()) };
    m_entry.entry(&1u64).set(&sample);
    let map_entry_slot = m_entry.slot_of(&1u64);
    // 4. StorageComponent::new_at + set
    {
        let mut lazy = <Lazy<V> as StorageComponent>::new_at(3, canonical, host.clone());
        lazy.set(&sample);
    }

    // --- Invariant 1: byte-identical slot contents ---
    let s_lazy = storage_get_32(&host, key_lazy_new.as_bytes());
    let s_map_insert = storage_get_32(&host, map_insert_slot.as_bytes());
    let s_map_entry = storage_get_32(&host, map_entry_slot.as_bytes());
    let s_component = storage_get_32(&host, key_component.as_bytes());
    assert_eq!(s_lazy, s_map_insert, "{name}: Lazy vs Mapping::insert");
    assert_eq!(s_map_insert, s_map_entry, "{name}: insert vs entry().set");
    assert_eq!(
        s_map_entry, s_component,
        "{name}: entry().set vs StorageComponent::new_at",
    );

    // --- Invariant 3: solc canonical placement ---
    let off = 32 - V::PACKED_BYTES;
    assert_eq!(
        &s_map_insert[off..32],
        tail,
        "{name}: value at canonical bytes {off}..32",
    );
    if off > 0 {
        assert!(
            s_map_insert[..off].iter().all(|&b| b == 0),
            "{name}: bytes 0..{off} should be zero (no neighbour)",
        );
    }

    // --- Invariant 2: every read path returns the sample ---
    let r_lazy = unsafe { Lazy::<V>::new(key_lazy_new, canonical, host.clone()) }.get();
    let r_map_insert = m_insert.get(&1u64);
    let r_map_entry_get = m_entry.get(&1u64);
    let r_map_entry_entry_get = m_entry.entry(&1u64).get();
    let r_component = <Lazy<V> as StorageComponent>::new_at(3, canonical, host.clone()).get();
    assert_eq!(r_lazy, sample, "{name}: Lazy round-trip");
    assert_eq!(r_map_insert, sample, "{name}: Mapping::get round-trip");
    assert_eq!(
        r_map_entry_get, sample,
        "{name}: entry().set then Mapping::get"
    );
    assert_eq!(
        r_map_entry_entry_get, sample,
        "{name}: entry().set then entry().get",
    );
    assert_eq!(r_component, sample, "{name}: StorageComponent round-trip");

    // --- Clear parity: every clear surface zeros the canonical window ---
    m_insert.remove(&1u64);
    assert_eq!(
        storage_get_32(&host, map_insert_slot.as_bytes()),
        [0u8; 32],
        "{name}: Mapping::remove auto-deletes",
    );
    m_entry.entry(&1u64).clear();
    assert_eq!(
        storage_get_32(&host, map_entry_slot.as_bytes()),
        [0u8; 32],
        "{name}: entry().clear auto-deletes (no neighbour)",
    );
    let mut lazy_clear = unsafe { Lazy::<V>::new(key_lazy_new, canonical, host.clone()) };
    lazy_clear.clear();
    assert_eq!(
        storage_get_32(&host, key_lazy_new.as_bytes()),
        [0u8; 32],
        "{name}: Lazy::clear auto-deletes",
    );
}

// Integer sweep (`tail` is the value's big-endian wire bytes).
#[test]
fn packing_parity_u8() {
    check_packing_parity::<u8>("u8", 0x42, &[0x42]);
}
#[test]
fn packing_parity_u16() {
    check_packing_parity::<u16>("u16", 0x1234, &0x1234u16.to_be_bytes());
}
#[test]
fn packing_parity_u32() {
    check_packing_parity::<u32>("u32", 0xDEAD_BEEF, &0xDEAD_BEEFu32.to_be_bytes());
}
#[test]
fn packing_parity_u64() {
    check_packing_parity::<u64>(
        "u64",
        0x0102_0304_0506_0708,
        &0x0102_0304_0506_0708u64.to_be_bytes(),
    );
}
#[test]
fn packing_parity_u128() {
    check_packing_parity::<u128>(
        "u128",
        0xCAFE_BABE_DEAD_BEEF,
        &0xCAFE_BABE_DEAD_BEEFu128.to_be_bytes(),
    );
}
#[test]
fn packing_parity_u256_full_slot() {
    check_packing_parity::<U256>(
        "U256",
        U256::from(0xFEEDu64),
        &U256::from(0xFEEDu64).to_be_bytes::<32>(),
    );
}

// Signed-integer sweep — exercises two's-complement encoding under the
// packed RMW path. Each tail must be the value's own big-endian bytes,
// NOT sign-extended across the slot (solc does not sign-extend across the
// canonical window for sub-word signed values).
#[test]
fn packing_parity_i8_negative() {
    check_packing_parity::<i8>("i8(-1)", -1, &(-1i8).to_be_bytes());
}
#[test]
fn packing_parity_i8_min() {
    check_packing_parity::<i8>("i8::MIN", i8::MIN, &i8::MIN.to_be_bytes());
}
#[test]
fn packing_parity_i16_negative() {
    check_packing_parity::<i16>("i16", -1234, &(-1234i16).to_be_bytes());
}
#[test]
fn packing_parity_i32_negative() {
    check_packing_parity::<i32>("i32", -0x1234_5678, &(-0x1234_5678i32).to_be_bytes());
}
#[test]
fn packing_parity_i64_min() {
    check_packing_parity::<i64>("i64::MIN", i64::MIN, &i64::MIN.to_be_bytes());
}
#[test]
fn packing_parity_i128_negative() {
    check_packing_parity::<i128>("i128", -42, &(-42i128).to_be_bytes());
}

// Other primitives.
#[test]
fn packing_parity_bool() {
    check_packing_parity::<bool>("bool", true, &[1]);
}
#[test]
fn packing_parity_address() {
    check_packing_parity::<Address>("Address", Address([0x42; 20]), &[0x42; 20]);
}

// bytesN at various N — right-aligned at bytes [32-N..32].
#[test]
fn packing_parity_bytes1() {
    check_packing_parity::<[u8; 1]>("bytes1", [0x42], &[0x42]);
}
#[test]
fn packing_parity_bytes4() {
    check_packing_parity::<[u8; 4]>(
        "bytes4",
        [0xDE, 0xAD, 0xBE, 0xEF],
        &[0xDE, 0xAD, 0xBE, 0xEF],
    );
}
#[test]
fn packing_parity_bytes20() {
    let v: [u8; 20] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10, 0x11, 0x12, 0x13, 0x14,
    ];
    check_packing_parity::<[u8; 20]>("bytes20", v, &v);
}
#[test]
fn packing_parity_bytes31() {
    let v: [u8; 31] = core::array::from_fn(|i| (i + 1) as u8);
    check_packing_parity::<[u8; 31]>("bytes31", v, &v);
}
#[test]
fn packing_parity_bytes32_full_slot() {
    let v: [u8; 32] = core::array::from_fn(|i| i as u8);
    check_packing_parity::<[u8; 32]>("bytes32", v, &v);
}

/// Wire-level: `i8::-1` packs as 0xFF at the canonical byte with bytes
/// above zero — solc does NOT sign-extend across the slot.
#[test]
fn signed_negative_packs_with_no_sign_extension() {
    let host = h();
    let mut m = unsafe { Mapping::<u64, i8>::new(StorageKey::from_slot(0), host.clone()) };
    m.insert(&1u64, &-1i8);
    let s = storage_get_32(&host, m.slot_of(&1u64).as_bytes());
    assert_eq!(s[31], 0xFF, "i8(-1) at canonical byte 31");
    assert!(
        s[..31].iter().all(|&b| b == 0),
        "no sign extension across the slot",
    );
}

/// Cross-path clobber: `insert` then `entry().clear` actually clears.
/// Catches the original bug (clear-at-wrong-offset would leave the
/// insert-written value intact at solc's canonical offset).
#[test]
fn insert_then_entry_clear_actually_clears_u128() {
    let host = h();
    let mut m = unsafe { Mapping::<u64, u128>::new(StorageKey::from_slot(0), host.clone()) };
    m.insert(&1u64, &0xDEAD_BEEFu128);
    m.entry(&1u64).clear();
    assert_eq!(m.get(&1u64), 0);
    assert_eq!(
        storage_get_32(&host, m.slot_of(&1u64).as_bytes()),
        [0u8; 32],
        "slot must be auto-deleted",
    );
}

/// Cross-path overwrite: `entry().set(a)` then `insert(b)` must leave
/// only `b` in the slot at the canonical offset (no residual `a` bytes
/// at a different position).
#[test]
fn entry_set_then_insert_overwrites_address() {
    let host = h();
    let mut m = unsafe { Mapping::<u64, Address>::new(StorageKey::from_slot(0), host.clone()) };
    let a = Address([0xAA; 20]);
    let b = Address([0xBB; 20]);
    m.entry(&1u64).set(&a);
    m.insert(&1u64, &b);
    assert_eq!(m.get(&1u64), b);
    let s = storage_get_32(&host, m.slot_of(&1u64).as_bytes());
    assert_eq!(&s[12..32], &b.0, "Address at canonical bytes 12..32");
    assert!(s[..12].iter().all(|&x| x == 0), "no residual bytes above");
}

// --- Multi-field storage ---

#[test]
fn multi_field_storage() {
    let host = h();
    let mut counter = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, host.clone()) };
    let mut balances = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(1), host) };

    counter.set(&U256::from(42));
    assert_eq!(counter.get(), U256::from(42));

    let addr = Address([0xFF; 20]);
    balances.insert(&addr, &U256::from(1000));
    assert_eq!(balances.get(&addr), U256::from(1000));
}

/// Full ERC-20-like example showing how storage fields are constructed
/// and used. This mirrors the `#[contract]` macro's generated code.
#[test]
fn erc20_storage_example() {
    let host = h();
    let mut total_supply = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, host.clone()) };
    let mut balances =
        unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(1), host.clone()) };
    let mut allowances =
        unsafe { Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), host) };

    let alice = Address([0xAA; 20]);
    let bob = Address([0xBB; 20]);
    let initial_supply = U256::from(10_000);

    // Constructor: set total supply and mint to alice
    total_supply.set(&initial_supply);
    balances.insert(&alice, &initial_supply);

    assert_eq!(total_supply.get(), initial_supply);
    assert_eq!(balances.get(&alice), initial_supply);
    assert_eq!(balances.get(&bob), U256::ZERO);

    // Transfer: alice sends 300 to bob using entry() for read-then-write
    let amount = U256::from(300);
    let mut alice_cell = balances.entry(&alice);
    let alice_bal = alice_cell.get();
    alice_cell.set(&(alice_bal - amount));

    let mut bob_cell = balances.entry(&bob);
    let bob_bal = bob_cell.get();
    bob_cell.set(&(bob_bal + amount));

    assert_eq!(balances.get(&alice), U256::from(9_700));
    assert_eq!(balances.get(&bob), U256::from(300));

    // Approve: alice approves bob for 500
    allowances.entry(&alice).insert(&bob, &U256::from(500));

    // Read allowance via chaining
    assert_eq!(allowances.get(&alice).get(&bob), U256::from(500));
    // Other direction is zero
    assert_eq!(allowances.get(&bob).get(&alice), U256::ZERO);
}

#[test]
fn different_slots_dont_interfere() {
    let host = h();
    let mut value_a = unsafe { Lazy::<U256>::new(StorageKey::from_slot(5), 0, host.clone()) };
    let mut value_b = unsafe { Lazy::<U256>::new(StorageKey::from_slot(10), 0, host) };

    value_a.set(&U256::from(111));
    value_b.set(&U256::from(222));
    assert_eq!(value_a.get(), U256::from(111));
    assert_eq!(value_b.get(), U256::from(222));
}

// --- Solidity slot cross-checks (hardcoded values from `cast index`) ---

#[test]
fn mapping_solidity_slot_compat() {
    // `cast index address 0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB 1`
    // → 0x8f22848572deaf321ecb41095a0a57d3f19eda24b92a3f4a8e554a2e56f45bc4
    let m = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(1), h()) };
    let addr = Address([0xBB; 20]);
    let slot = m.slot_of(&addr);

    let expected = [
        0x8f, 0x22, 0x84, 0x85, 0x72, 0xde, 0xaf, 0x32, 0x1e, 0xcb, 0x41, 0x09, 0x5a, 0x0a, 0x57,
        0xd3, 0xf1, 0x9e, 0xda, 0x24, 0xb9, 0x2a, 0x3f, 0x4a, 0x8e, 0x55, 0x4a, 0x2e, 0x56, 0xf4,
        0x5b, 0xc4,
    ];
    assert_eq!(slot.as_bytes(), &expected, "must match `cast index` output");
}

#[test]
fn nested_mapping_slot_matches_solidity() {
    // allowances[0xAA..AA][0xBB..BB] at root slot 2:
    // inner = cast index address 0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA 2
    //       → 0xe1e81504ed8609a5b03379f97b221e3dede4a62d6d61a87a4ab7ed7b1b9c0553
    // outer = cast index address 0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB <inner>
    //       → 0x35815c850ac7d4d0af322824699787b146e33c6cac5d0a52ab3225d6985a27a7
    let allowances =
        unsafe { Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), h()) };
    let owner = Address([0xAA; 20]);
    let spender = Address([0xBB; 20]);

    // Derive via chaining: get(&owner) returns inner Mapping, then slot_of(&spender)
    let inner = allowances.get(&owner);
    let slot = inner.slot_of(&spender);

    let expected = [
        0x35, 0x81, 0x5c, 0x85, 0x0a, 0xc7, 0xd4, 0xd0, 0xaf, 0x32, 0x28, 0x24, 0x69, 0x97, 0x87,
        0xb1, 0x46, 0xe3, 0x3c, 0x6c, 0xac, 0x5d, 0x0a, 0x52, 0xab, 0x32, 0x25, 0xd6, 0x98, 0x5a,
        0x27, 0xa7,
    ];
    assert_eq!(
        slot.as_bytes(),
        &expected,
        "must match chained `cast index` output"
    );
}

// --- Dynamic keys (String / Vec<u8>) ---
// Run with: cargo test -p pvm-storage --features alloc

#[cfg(feature = "alloc")]
use alloc::string::ToString;
#[cfg(feature = "alloc")]
use alloc::vec;

#[cfg(feature = "alloc")]
#[test]
fn mapping_string_key_roundtrip() {
    let mut m = unsafe { Mapping::<String, U256>::new(StorageKey::from_slot(0), h()) };
    m.insert(&"alice".to_string(), &U256::from(100));
    assert_eq!(m.get(&"alice".to_string()), U256::from(100));
    assert_eq!(m.get(&"bob".to_string()), U256::ZERO);
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_bytes_key_roundtrip() {
    let mut m = unsafe { Mapping::<Vec<u8>, U256>::new(StorageKey::from_slot(0), h()) };
    m.insert(&vec![1u8, 2, 3], &U256::from(42));
    assert_eq!(m.get(&vec![1u8, 2, 3]), U256::from(42));
    assert_eq!(m.get(&vec![1u8, 2, 4]), U256::ZERO);
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_bytes_key_long_roundtrip() {
    // 100-byte key spans multiple keccak preimage bytes; confirms the
    // unpadded formula handles arbitrary-length keys.
    let mut m = unsafe { Mapping::<Vec<u8>, U256>::new(StorageKey::from_slot(1), h()) };
    let key = vec![b'x'; 100];
    m.insert(&key, &U256::from(7));
    assert_eq!(m.get(&key), U256::from(7));
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_string_key_solidity_parity() {
    // cast index string "foo" 1
    // → 0xb770ea6769bbbd870e326681074f882a4d98de2943bbf7a23e8f4b258b1b8ac9
    let m = unsafe { Mapping::<String, U256>::new(StorageKey::from_slot(1), h()) };
    let slot = m.slot_of(&"foo".to_string());
    let expected = [
        0xb7, 0x70, 0xea, 0x67, 0x69, 0xbb, 0xbd, 0x87, 0x0e, 0x32, 0x66, 0x81, 0x07, 0x4f, 0x88,
        0x2a, 0x4d, 0x98, 0xde, 0x29, 0x43, 0xbb, 0xf7, 0xa2, 0x3e, 0x8f, 0x4b, 0x25, 0x8b, 0x1b,
        0x8a, 0xc9,
    ];
    assert_eq!(
        slot.as_bytes(),
        &expected,
        "must match `cast index string \"foo\" 1`"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_bytes_key_solidity_parity() {
    // cast index bytes "0x010203" 1
    // → 0x4c6b2a1cad5eaf1e4e6556e0d021d6a22514b15458a60294869177950c245b57
    let m = unsafe { Mapping::<Vec<u8>, U256>::new(StorageKey::from_slot(1), h()) };
    let slot = m.slot_of(&vec![1u8, 2, 3]);
    let expected = [
        0x4c, 0x6b, 0x2a, 0x1c, 0xad, 0x5e, 0xaf, 0x1e, 0x4e, 0x65, 0x56, 0xe0, 0xd0, 0x21, 0xd6,
        0xa2, 0x25, 0x14, 0xb1, 0x54, 0x58, 0xa6, 0x02, 0x94, 0x86, 0x91, 0x77, 0x95, 0x0c, 0x24,
        0x5b, 0x57,
    ];
    assert_eq!(
        slot.as_bytes(),
        &expected,
        "must match `cast index bytes \"0x010203\" 1`"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_string_key_empty() {
    // Empty key: preimage is just the 32-byte root slot.
    // keccak256(b"" ++ pad32(1)) = b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6
    let mut m = unsafe { Mapping::<String, U256>::new(StorageKey::from_slot(1), h()) };
    m.insert(&String::new(), &U256::from(9));
    assert_eq!(m.get(&String::new()), U256::from(9));

    let slot = m.slot_of(&String::new());
    let expected = [
        0xb1, 0x0e, 0x2d, 0x52, 0x76, 0x12, 0x07, 0x3b, 0x26, 0xee, 0xcd, 0xfd, 0x71, 0x7e, 0x6a,
        0x32, 0x0c, 0xf4, 0x4b, 0x4a, 0xfa, 0xc2, 0xb0, 0x73, 0x2d, 0x9f, 0xcb, 0xe2, 0xb7, 0xfa,
        0x0c, 0xf6,
    ];
    assert_eq!(slot.as_bytes(), &expected);
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_string_key_no_padding_collision_safety() {
    // The 1-byte string "a" (raw bytes: [0x61]) and the 32-byte static key
    // [0x61, 0x00*31] both have 0x61 as their first preimage byte. With the
    // padded formula they would collide; with the unpadded formula they
    // must NOT collide.
    let host = h();
    let dyn_map = unsafe { Mapping::<String, U256>::new(StorageKey::from_slot(0), host.clone()) };
    let static_map =
        unsafe { Mapping::<[u8; 32], U256>::new(StorageKey::from_slot(0), host.clone()) };

    let dyn_slot = dyn_map.slot_of(&"a".to_string());

    let mut padded_a = [0u8; 32];
    padded_a[0] = 0x61;
    let static_slot = static_map.slot_of(&padded_a);

    assert_ne!(
        dyn_slot.as_bytes(),
        static_slot.as_bytes(),
        "dynamic and static keys with shared prefix must derive distinct slots"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_string_key_distinct_lengths() {
    // "a" and "aa" share a prefix; verify distinct slots.
    let m = unsafe { Mapping::<String, U256>::new(StorageKey::from_slot(0), h()) };
    assert_ne!(
        m.slot_of(&"a".to_string()).as_bytes(),
        m.slot_of(&"aa".to_string()).as_bytes(),
    );
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_string_key_matches_str_impl() {
    // The String impl must delegate to the str impl so that derived slots
    // are byte-identical. This guarantee is what would let a future
    // `get_by_str` zero-alloc accessor share storage with the String API.
    let host = h();
    let root = StorageKey::from_slot(3);
    let m = unsafe { Mapping::<String, U256>::new(root, host.clone()) };
    let owned_slot = m.slot_of(&"alice".to_string());
    let borrowed_slot = <str as AsStorageKey>::derive_slot("alice", &host, &root);
    assert_eq!(owned_slot.as_bytes(), borrowed_slot.as_bytes());
}

// ---------------------------------------------------------------------
// Native String / Bytes in Lazy / Mapping
// ---------------------------------------------------------------------

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_native_short_round_trip() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    lazy.set(&String::from("hello"));
    assert_eq!(lazy.get(), "hello");
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_native_long_round_trip() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let long: String = "x".repeat(80); // spills across multiple body chunks
    lazy.set(&long);
    assert_eq!(lazy.get(), long);
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_native_try_get_distinguishes_set_empty_from_unset() {
    let mut written = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let never = unsafe { Lazy::<String>::new(StorageKey::from_slot(1), 0, written.host.clone()) };

    written.set(&String::new());
    let got = written.try_get();
    assert_eq!(got, Some(String::new()));
    assert!(never.try_get().is_none());
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_native_clear_removes_header_and_body() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;

    lazy.set(&"x".repeat(80));
    lazy.clear();

    assert_eq!(
        storage_try_get_32(&host, key.as_bytes()),
        None,
        "header not cleared"
    );
    let mut body = dynamic_data_root(&host, key.as_bytes());
    for _ in 0..3 {
        assert_eq!(storage_try_get_32(&host, &body), None);
        inc_slot(&mut body);
    }
}

#[cfg(feature = "alloc")]
#[test]
fn mapping_string_native_round_trip() {
    let mut m = unsafe { Mapping::<u64, String>::new(StorageKey::from_slot(0), h()) };
    m.insert(&1u64, &String::from("hello"));
    m.insert(&2u64, &"y".repeat(64));

    assert_eq!(m.get(&1u64), "hello");
    assert_eq!(m.get(&2u64), "y".repeat(64));
    assert!(m.try_get(&3u64).is_none());

    m.remove(&1u64);
    assert!(m.try_get(&1u64).is_none());
    assert_eq!(m.get(&2u64), "y".repeat(64));
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_bytes_native_round_trip() {
    let mut lazy = unsafe { Lazy::<Bytes>::new(StorageKey::from_slot(0), 0, h()) };
    let payload = Bytes((0..50).collect());
    lazy.set(&payload);
    assert_eq!(lazy.get(), payload);
}

#[cfg(feature = "alloc")]
#[test]
fn lazy_string_native_layout_matches_solc_short() {
    let mut lazy = unsafe { Lazy::<String>::new(StorageKey::from_slot(0), 0, h()) };
    let host = lazy.host.clone();
    let key = lazy.key;
    lazy.set(&String::from("hello"));

    let header = storage_get_32(&host, key.as_bytes());
    assert_eq!(&header[..5], b"hello");
    assert!(header[5..31].iter().all(|&b| b == 0));
    assert_eq!(header[31], 5 * 2);
}

// --- StorageVec ---

#[test]
fn storage_vec_empty_defaults() {
    let v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    assert_eq!(v.len(), 0);
    assert!(v.is_empty());
    assert_eq!(v.try_get(0), None);
}

#[test]
fn storage_vec_push_pop_u256() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    v.push(&U256::from(10u64));
    v.push(&U256::from(20u64));
    v.push(&U256::from(30u64));
    assert_eq!(v.len(), 3);
    assert_eq!(v.get(0), U256::from(10u64));
    assert_eq!(v.get(1), U256::from(20u64));
    assert_eq!(v.get(2), U256::from(30u64));

    assert_eq!(v.pop(), Some(U256::from(30u64)));
    assert_eq!(v.pop(), Some(U256::from(20u64)));
    assert_eq!(v.len(), 1);
    assert_eq!(v.pop(), Some(U256::from(10u64)));
    assert_eq!(v.len(), 0);
    assert_eq!(v.pop(), None);
}

#[test]
fn storage_vec_push_pop_address() {
    let mut v = unsafe { StorageVec::<Address>::new(StorageKey::from_slot(7), h()) };
    let a = Address([0xAA; 20]);
    let b = Address([0xBB; 20]);
    v.push(&a);
    v.push(&b);
    assert_eq!(v.get(0), a);
    assert_eq!(v.get(1), b);
    assert_eq!(v.pop(), Some(b));
    assert_eq!(v.pop(), Some(a));
    assert_eq!(v.pop(), None);
}

#[test]
fn storage_vec_set_overwrites() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    v.push(&U256::from(1u64));
    v.push(&U256::from(2u64));
    v.set(0, &U256::from(100u64));
    assert_eq!(v.get(0), U256::from(100u64));
    assert_eq!(v.get(1), U256::from(2u64));
}

#[test]
fn storage_vec_try_get_oob_returns_none() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    v.push(&U256::from(42u64));
    assert_eq!(v.try_get(0), Some(U256::from(42u64)));
    assert_eq!(v.try_get(1), None);
    assert_eq!(v.try_get(u64::MAX), None);
}

#[test]
fn storage_vec_first_last() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    assert_eq!(v.first(), None);
    assert_eq!(v.last(), None);
    v.push(&U256::from(10u64));
    v.push(&U256::from(20u64));
    v.push(&U256::from(30u64));
    assert_eq!(v.first(), Some(U256::from(10u64)));
    assert_eq!(v.last(), Some(U256::from(30u64)));
}

#[test]
fn storage_vec_iter_yields_all_in_order() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    for i in 0..5u64 {
        v.push(&U256::from(i * 100));
    }
    let collected: alloc::vec::Vec<U256> = v.iter().collect();
    assert_eq!(
        collected,
        alloc::vec![
            U256::from(0u64),
            U256::from(100u64),
            U256::from(200u64),
            U256::from(300u64),
            U256::from(400u64),
        ]
    );
    // ExactSizeIterator length.
    assert_eq!(v.iter().len(), 5);
    // Empty vec yields nothing.
    let empty = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(9), h()) };
    assert_eq!(empty.iter().next(), None);
}

#[test]
fn storage_vec_iter_reverse() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    for i in 0..4u64 {
        v.push(&U256::from(i));
    }
    let collected: alloc::vec::Vec<U256> = v.iter().rev().collect();
    assert_eq!(
        collected,
        alloc::vec![
            U256::from(3u64),
            U256::from(2u64),
            U256::from(1u64),
            U256::from(0u64),
        ]
    );
}

#[test]
#[should_panic(expected = "out of bounds")]
fn storage_vec_get_oob_panics() {
    let v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    let _ = v.get(0);
}

#[test]
#[should_panic(expected = "out of bounds")]
fn storage_vec_set_oob_panics() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    v.set(0, &U256::from(1u64));
}

#[test]
fn storage_vec_clear_resets_everything() {
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    let host = v.host.clone();
    let root = v.root;
    v.push(&U256::from(1u64));
    v.push(&U256::from(2u64));
    v.push(&U256::from(3u64));

    // Capture the body base via the keccak path the same way the impl does.
    let mut body = [0u8; 32];
    host.hash_keccak_256(root.as_bytes(), &mut body);

    v.clear();
    assert_eq!(v.len(), 0);
    assert!(v.is_empty());

    // Length slot is fully zeroed.
    assert_eq!(storage_get_32(&host, root.as_bytes()), [0u8; 32]);

    // Each former element slot is also zeroed (no stale data left behind).
    let mut k = body;
    for _ in 0..3 {
        assert_eq!(storage_get_32(&host, &k), [0u8; 32]);
        inc_slot(&mut k);
    }
}

#[test]
fn storage_vec_pop_zeros_freed_slot() {
    // Pop must clear the freed slot so a later push doesn't see stale
    // bytes — and so the SSTORE-to-zero refund applies (matches solc).
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    let host = v.host.clone();
    v.push(&U256::from(0xABCDu64));

    // The element lives at keccak256(slot_0) + 0 = keccak256(slot_0).
    let mut body = [0u8; 32];
    host.hash_keccak_256(v.root.as_bytes(), &mut body);
    assert_ne!(storage_get_32(&host, &body), [0u8; 32]);

    let _ = v.pop();
    assert_eq!(
        storage_get_32(&host, &body),
        [0u8; 32],
        "pop must zero the freed slot",
    );
}

#[test]
fn storage_vec_layout_matches_solidity_uint256_array() {
    // Solidity layout for `uint256[] a;` at slot 5:
    //   slot[5] = length
    //   element i = slot[ keccak256(pad32(5)) + i ]
    //
    // Cross-check by writing through StorageVec and reading raw slots.
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(5), h()) };
    let host = v.host.clone();
    v.push(&U256::from(0xAAu64));
    v.push(&U256::from(0xBBu64));
    v.push(&U256::from(0xCCu64));

    // Length lives at the root slot, big-endian uint256.
    let len_slot = storage_get_32(&host, v.root.as_bytes());
    assert_eq!(len_slot[31], 3);
    assert!(len_slot[..31].iter().all(|&b| b == 0));

    // Body base is keccak256(pad32(5)).
    let mut body = [0u8; 32];
    host.hash_keccak_256(v.root.as_bytes(), &mut body);

    // Elements at body + 0, body + 1, body + 2.
    let e0 = storage_get_32(&host, &body);
    assert_eq!(e0[31], 0xAA);
    let mut k = body;
    inc_slot(&mut k);
    let e1 = storage_get_32(&host, &k);
    assert_eq!(e1[31], 0xBB);
    inc_slot(&mut k);
    let e2 = storage_get_32(&host, &k);
    assert_eq!(e2[31], 0xCC);
}

#[test]
fn storage_vec_body_base_independent_of_other_slots() {
    // Two arrays at different slots must not collide.
    let host = h();
    let mut v0 = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), host.clone()) };
    let mut v1 = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(1), host.clone()) };
    v0.push(&U256::from(111u64));
    v1.push(&U256::from(222u64));
    assert_eq!(v0.get(0), U256::from(111u64));
    assert_eq!(v1.get(0), U256::from(222u64));
    assert_eq!(v0.len(), 1);
    assert_eq!(v1.len(), 1);
}

#[test]
fn storage_vec_get_after_set_reuses_body_base_cache() {
    // Smoke test the OnceCell: many element accesses on the same handle.
    let mut v = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), h()) };
    for i in 0..16u64 {
        v.push(&U256::from(i + 100));
    }
    for i in 0..16u64 {
        assert_eq!(v.get(i), U256::from(i + 100));
    }
}

#[test]
fn storage_vec_storage_component_metadata() {
    assert_eq!(<StorageVec<U256> as StorageComponent>::SLOTS, 1);
    assert_eq!(<StorageVec<U256> as StorageComponent>::PACKED_BYTES, 32);
    assert_eq!(<StorageVec<Address> as StorageComponent>::SLOTS, 1);
    assert_eq!(<StorageVec<Address> as StorageComponent>::PACKED_BYTES, 32);
}

#[test]
fn storage_vec_new_at_matches_unsafe_new() {
    // `StorageComponent::new_at` is the macro-emitted safe path; assert
    // it produces a handle indistinguishable from `unsafe { new(...) }`.
    let host = h();
    let mut a = unsafe { StorageVec::<U256>::new(StorageKey::from_slot(3), host.clone()) };
    let mut b = <StorageVec<U256> as StorageComponent>::new_at(3, 0, host);
    a.push(&U256::from(7u64));
    assert_eq!(b.get(0), U256::from(7u64));
    b.push(&U256::from(9u64));
    assert_eq!(a.get(1), U256::from(9u64));
}

// --- StorageVec<T> for sub-word T (uint32[], uint64[], bool[], etc.) ---

#[test]
fn storage_vec_subword_u32_roundtrip() {
    let mut v = unsafe { StorageVec::<u32>::new(StorageKey::from_slot(0), h()) };
    for i in 0u32..20 {
        v.push(&(i * 100));
    }
    assert_eq!(v.len(), 20);
    for i in 0u32..20 {
        assert_eq!(v.get(i as u64), i * 100);
    }
    // Overwrite a middle element via `set`.
    v.set(5, &9999u32);
    assert_eq!(v.get(5), 9999u32);
    // Neighbours in the same packed slot are preserved.
    assert_eq!(v.get(4), 400u32);
    assert_eq!(v.get(6), 600u32);
}

#[test]
fn storage_vec_subword_u32_layout_matches_solc() {
    // solc `uint32[]` at slot S:
    //   length at S
    //   element i at slot keccak(pad32(S)) + (i / 8), bytes 32 - 4*(within+1)..
    //   where within = i % 8 (per_slot = 32 / 4 = 8)
    let host = h();
    let mut v = unsafe { StorageVec::<u32>::new(StorageKey::from_slot(2), host.clone()) };
    for i in 0u32..10 {
        v.push(&(0xC0DE_0000 | i));
    }
    let body = storage_derive_body_base(&host, StorageKey::from_slot(2).as_bytes());

    // Slot 0 of body holds elements 0..8 (right-aligned: 0 lowest, 7 highest).
    let s0 = storage_get_32(&host, &body);
    for within in 0u32..8 {
        let off = 32 - 4 * (within as usize + 1);
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&s0[off..off + 4]);
        assert_eq!(u32::from_be_bytes(bytes), 0xC0DE_0000 | within);
    }

    // Slot 1 of body holds elements 8..10.
    let mut s1_key = body;
    inc_slot(&mut s1_key);
    let s1 = storage_get_32(&host, &s1_key);
    // Element 8 lives at within=0 (bytes 28..32).
    assert_eq!(
        u32::from_be_bytes([s1[28], s1[29], s1[30], s1[31]]),
        0xC0DE_0008
    );
    // Element 9 at within=1 (bytes 24..28).
    assert_eq!(
        u32::from_be_bytes([s1[24], s1[25], s1[26], s1[27]]),
        0xC0DE_0009
    );
    // No data above element 9 (high bytes of slot 1 are zero).
    assert_eq!(&s1[..24], &[0u8; 24]);
}

#[test]
fn storage_vec_subword_bool_packs_32_per_slot() {
    // `bool` has PACKED_BYTES = 1, so 32 bools fit per slot.
    let mut v = unsafe { StorageVec::<bool>::new(StorageKey::from_slot(0), h()) };
    for i in 0..40 {
        v.push(&(i % 3 == 0));
    }
    for i in 0..40u64 {
        assert_eq!(v.get(i), (i % 3) == 0);
    }
}

#[test]
fn storage_vec_subword_pop_preserves_packed_siblings() {
    // After pushing 5 u32s and popping the last one, the remaining 4 must
    // still be readable — pop's RMW must zero only the freed bytes.
    let mut v = unsafe { StorageVec::<u32>::new(StorageKey::from_slot(0), h()) };
    for i in 0u32..5 {
        v.push(&(0x1000 + i));
    }
    assert_eq!(v.pop(), Some(0x1004));
    assert_eq!(v.len(), 4);
    for i in 0u32..4 {
        assert_eq!(v.get(i as u64), 0x1000 + i);
    }
}

#[test]
fn storage_vec_subword_pop_clears_slot_when_freeing_first_in_slot() {
    // u32 with per_slot = 8. Push 9 elements (slot 0: indices 0..8, slot 1:
    // index 8). Popping index 8 should clear slot 1 entirely — verify via
    // raw read that the slot is zero.
    let host = h();
    let mut v = unsafe { StorageVec::<u32>::new(StorageKey::from_slot(0), host.clone()) };
    for i in 0u32..9 {
        v.push(&(i + 1));
    }
    let body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let mut slot1_key = body;
    inc_slot(&mut slot1_key);
    // Before pop: slot 1 has element 8 at bytes 28..32.
    let before = storage_get_32(&host, &slot1_key);
    assert_eq!(
        u32::from_be_bytes([before[28], before[29], before[30], before[31]]),
        9
    );

    assert_eq!(v.pop(), Some(9));

    // After pop: slot 1 is fully cleared (it had only one element).
    let after = storage_get_32(&host, &slot1_key);
    assert_eq!(after, [0u8; 32]);

    // And slot 0 still has all 8 surviving elements packed.
    let slot0 = storage_get_32(&host, &body);
    assert_ne!(slot0, [0u8; 32]);
}

#[test]
fn storage_vec_subword_clear_resets_all_body_slots() {
    let host = h();
    let mut v = unsafe { StorageVec::<u32>::new(StorageKey::from_slot(0), host.clone()) };
    for i in 0u32..17 {
        // 17 u32s span 3 slots (8 + 8 + 1).
        v.push(&(i + 1));
    }
    v.clear();
    assert_eq!(v.len(), 0);

    // Verify all 3 body slots are zero.
    let body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let mut key = body;
    for _ in 0..3 {
        assert_eq!(storage_get_32(&host, &key), [0u8; 32]);
        inc_slot(&mut key);
    }
}

#[test]
fn storage_vec_subword_storage_component_metadata() {
    // Sub-word StorageVecs report the same metadata as full-word ones:
    // one root slot, never packs with neighbours.
    assert_eq!(<StorageVec<u32> as StorageComponent>::SLOTS, 1);
    assert_eq!(<StorageVec<u32> as StorageComponent>::PACKED_BYTES, 32);
    assert_eq!(<StorageVec<u64> as StorageComponent>::SLOTS, 1);
    assert_eq!(<StorageVec<bool> as StorageComponent>::SLOTS, 1);
}

// --- StorageVec<T> for multi-slot static T ((U256, U256)[], etc.) ---

#[test]
fn storage_vec_multislot_tuple_roundtrip() {
    // `(U256, U256)` has STORAGE_SLOTS == 2 — each element claims two slots.
    let mut v = unsafe { StorageVec::<(U256, U256)>::new(StorageKey::from_slot(0), h()) };
    let pairs: [(U256, U256); 4] = [
        (U256::from(1u64), U256::from(2u64)),
        (U256::from(3u64), U256::from(4u64)),
        (U256::from(5u64), U256::from(6u64)),
        (U256::from(7u64), U256::from(8u64)),
    ];
    for p in &pairs {
        v.push(p);
    }
    assert_eq!(v.len(), 4);
    for (i, expected) in pairs.iter().enumerate() {
        assert_eq!(v.get(i as u64), *expected);
    }
}

#[test]
fn storage_vec_multislot_layout_uses_stride() {
    // Verify that element i occupies slots [body_base + i*N, body_base + i*N + N).
    let host = h();
    let mut v = unsafe { StorageVec::<(U256, U256)>::new(StorageKey::from_slot(5), host.clone()) };
    v.push(&(U256::from(0x1111u64), U256::from(0x2222u64)));
    v.push(&(U256::from(0x3333u64), U256::from(0x4444u64)));

    let body = storage_derive_body_base(&host, StorageKey::from_slot(5).as_bytes());
    // Element 0 occupies slots body+0, body+1.
    let e0_s0 = storage_get_32(&host, &body);
    let mut k = body;
    inc_slot(&mut k);
    let e0_s1 = storage_get_32(&host, &k);
    // Tuple `(a, b)` encoding: a at slot 0, b at slot 1 (both right-aligned U256s).
    assert_eq!(U256::from_be_bytes(e0_s0), U256::from(0x1111u64));
    assert_eq!(U256::from_be_bytes(e0_s1), U256::from(0x2222u64));

    // Element 1 occupies slots body+2, body+3.
    inc_slot(&mut k);
    let e1_s0 = storage_get_32(&host, &k);
    inc_slot(&mut k);
    let e1_s1 = storage_get_32(&host, &k);
    assert_eq!(U256::from_be_bytes(e1_s0), U256::from(0x3333u64));
    assert_eq!(U256::from_be_bytes(e1_s1), U256::from(0x4444u64));
}

#[test]
fn storage_vec_multislot_set_overwrites() {
    let mut v = unsafe { StorageVec::<(U256, U256)>::new(StorageKey::from_slot(0), h()) };
    v.push(&(U256::from(1u64), U256::from(2u64)));
    v.push(&(U256::from(3u64), U256::from(4u64)));
    v.set(0, &(U256::from(99u64), U256::from(100u64)));
    assert_eq!(v.get(0), (U256::from(99u64), U256::from(100u64)));
    assert_eq!(v.get(1), (U256::from(3u64), U256::from(4u64)));
}

#[test]
fn storage_vec_multislot_pop_clears_all_slots() {
    let host = h();
    let mut v = unsafe { StorageVec::<(U256, U256)>::new(StorageKey::from_slot(0), host.clone()) };
    v.push(&(U256::from(7u64), U256::from(8u64)));
    v.push(&(U256::from(9u64), U256::from(10u64)));

    assert_eq!(v.pop(), Some((U256::from(9u64), U256::from(10u64))));
    assert_eq!(v.len(), 1);

    // Verify the freed element's slots are both zero.
    let body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let mut k = body;
    inc_slot_by(&mut k, 2); // element 1 starts at body + 2
    assert_eq!(storage_get_32(&host, &k), [0u8; 32]);
    inc_slot(&mut k);
    assert_eq!(storage_get_32(&host, &k), [0u8; 32]);
}

// --- StorageVec<T> for fixed-size arrays [T; N] (T != u8) ---

#[test]
fn storage_vec_fixed_array_u32_roundtrip() {
    // `uint32[4]` fits in one slot (4*4 = 16 bytes); the array's STORAGE_SLOTS
    // is 1, so each StorageVec element claims exactly one body slot.
    let mut v = unsafe { StorageVec::<[u32; 4]>::new(StorageKey::from_slot(0), h()) };
    v.push(&[10, 20, 30, 40]);
    v.push(&[1, 2, 3, 4]);
    assert_eq!(v.len(), 2);
    assert_eq!(v.get(0), [10, 20, 30, 40]);
    assert_eq!(v.get(1), [1, 2, 3, 4]);
}

#[test]
fn storage_vec_fixed_array_u32_boundary_crossing() {
    // `uint32[9]` spans 2 slots (8 elements + 1 spillover). Verify byte
    // placement matches solc:
    //   slot 0: elements 0..8 right-aligned (element i at bytes 28-4i .. 32-4i)
    //   slot 1: element 8 at bytes 28..32, rest zero
    let host = h();
    let mut v = unsafe { StorageVec::<[u32; 9]>::new(StorageKey::from_slot(0), host.clone()) };
    let arr: [u32; 9] = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18];
    v.push(&arr);

    let body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let slot0 = storage_get_32(&host, &body);
    // element 0 right-aligned in slot 0
    assert_eq!(
        u32::from_be_bytes([slot0[28], slot0[29], slot0[30], slot0[31]]),
        0x10
    );
    // element 7 at the high end of slot 0
    assert_eq!(
        u32::from_be_bytes([slot0[0], slot0[1], slot0[2], slot0[3]]),
        0x17
    );

    let mut slot1_key = body;
    inc_slot(&mut slot1_key);
    let slot1 = storage_get_32(&host, &slot1_key);
    // element 8 right-aligned in slot 1
    assert_eq!(
        u32::from_be_bytes([slot1[28], slot1[29], slot1[30], slot1[31]]),
        0x18
    );
    // high 28 bytes of slot 1 are zero
    assert_eq!(&slot1[..28], &[0u8; 28]);

    // Round-trip
    assert_eq!(v.get(0), arr);
}

#[test]
fn storage_vec_fixed_array_bool_packing() {
    // `bool[40]` spans 2 slots (32 + 8). Each bool is 1 byte, density 32.
    let mut v = unsafe { StorageVec::<[bool; 40]>::new(StorageKey::from_slot(0), h()) };
    let mut arr = [false; 40];
    for (i, slot) in arr.iter_mut().enumerate() {
        *slot = i % 3 == 0;
    }
    v.push(&arr);
    assert_eq!(v.get(0), arr);
}

#[test]
fn storage_vec_fixed_array_address_no_packing() {
    // `address[3]` — Address is 20 bytes, density = 32/20 = 1 (no packing
    // across addresses). Should consume 3 slots, one per element.
    assert_eq!(<[Address; 3] as StorageEncode>::STORAGE_SLOTS, 3);
    let host = h();
    let mut v = unsafe { StorageVec::<[Address; 3]>::new(StorageKey::from_slot(0), host.clone()) };
    let arr = [
        Address([0xAA; 20]),
        Address([0xBB; 20]),
        Address([0xCC; 20]),
    ];
    v.push(&arr);
    assert_eq!(v.get(0), arr);

    // Verify the 3 slots each hold one address right-aligned.
    let body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let mut k = body;
    for expected in arr.iter() {
        let slot = storage_get_32(&host, &k);
        assert_eq!(&slot[..12], &[0u8; 12]);
        assert_eq!(&slot[12..32], &expected.0);
        inc_slot(&mut k);
    }
}

#[test]
fn storage_vec_fixed_array_u256_full_slot() {
    // `uint256[3]` — full-slot T, 3 slots per element. STORAGE_SLOTS = 3.
    assert_eq!(<[U256; 3] as StorageEncode>::STORAGE_SLOTS, 3);
    let mut v = unsafe { StorageVec::<[U256; 3]>::new(StorageKey::from_slot(0), h()) };
    let arr = [
        U256::from(0xAAu64),
        U256::from(0xBBu64),
        U256::from(0xCCu64),
    ];
    v.push(&arr);
    v.push(&[U256::from(1u64), U256::from(2u64), U256::from(3u64)]);
    assert_eq!(v.get(0), arr);
    assert_eq!(
        v.get(1),
        [U256::from(1u64), U256::from(2u64), U256::from(3u64)]
    );
}

#[test]
fn storage_vec_fixed_array_pop_clears_all_slots() {
    // Popping an `[u32; 9]` must clear both its slots so a later push doesn't
    // see stale bytes — matches solc's `pop()` zeroing.
    let host = h();
    let mut v = unsafe { StorageVec::<[u32; 9]>::new(StorageKey::from_slot(0), host.clone()) };
    v.push(&[0xFF; 9]);

    let body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let mut slot1 = body;
    inc_slot(&mut slot1);
    assert_ne!(storage_get_32(&host, &body), [0u8; 32]);
    assert_ne!(storage_get_32(&host, &slot1), [0u8; 32]);

    let _ = v.pop();
    assert_eq!(v.len(), 0);
    assert_eq!(storage_get_32(&host, &body), [0u8; 32]);
    assert_eq!(storage_get_32(&host, &slot1), [0u8; 32]);
}

#[test]
fn storage_vec_fixed_array_storage_slots_metadata() {
    // Compile-time STORAGE_SLOTS for representative shapes.
    assert_eq!(<[u32; 4] as StorageEncode>::STORAGE_SLOTS, 1); // fits 1 slot
    assert_eq!(<[u32; 8] as StorageEncode>::STORAGE_SLOTS, 1); // exactly fills 1 slot
    assert_eq!(<[u32; 9] as StorageEncode>::STORAGE_SLOTS, 2); // spills to 2
    assert_eq!(<[u64; 4] as StorageEncode>::STORAGE_SLOTS, 1); // 4*8 = 32
    assert_eq!(<[u64; 5] as StorageEncode>::STORAGE_SLOTS, 2);
    assert_eq!(<[bool; 32] as StorageEncode>::STORAGE_SLOTS, 1);
    assert_eq!(<[bool; 33] as StorageEncode>::STORAGE_SLOTS, 2);
    assert_eq!(<[U256; 1] as StorageEncode>::STORAGE_SLOTS, 1);
    assert_eq!(<[U256; 5] as StorageEncode>::STORAGE_SLOTS, 5);
    assert_eq!(<[Address; 3] as StorageEncode>::STORAGE_SLOTS, 3);
    // PACKED_BYTES is always 32 — arrays start a fresh slot.
    assert_eq!(<[u32; 4] as StorageEncode>::PACKED_BYTES, 32);
}

// --- StorageVec<T> for dynamic-body T (string[], bytes[]) ---

#[cfg(feature = "alloc")]
#[test]
fn storage_vec_dynamic_string_roundtrip() {
    let mut v = unsafe { StorageVec::<String>::new(StorageKey::from_slot(0), h()) };
    v.push(&String::from("short"));
    // 32+ byte string forces the spilled-body path.
    v.push(&String::from(
        "this is a much longer string that exceeds 31 bytes",
    ));
    v.push(&String::new());
    v.push(&String::from("another"));

    assert_eq!(v.len(), 4);
    assert_eq!(v.get(0), "short");
    assert_eq!(
        v.get(1),
        "this is a much longer string that exceeds 31 bytes"
    );
    assert_eq!(v.get(2), "");
    assert_eq!(v.get(3), "another");
}

#[cfg(feature = "alloc")]
#[test]
fn storage_vec_dynamic_bytes_roundtrip() {
    let mut v = unsafe { StorageVec::<Bytes>::new(StorageKey::from_slot(0), h()) };
    v.push(&Bytes(alloc::vec![0xAA, 0xBB, 0xCC]));
    v.push(&Bytes(alloc::vec![0x11; 100])); // forces spill
    v.push(&Bytes(alloc::vec::Vec::new()));

    assert_eq!(v.len(), 3);
    assert_eq!(v.get(0).0, alloc::vec![0xAA, 0xBB, 0xCC]);
    assert_eq!(v.get(1).0, alloc::vec![0x11; 100]);
    assert_eq!(v.get(2).0, alloc::vec::Vec::<u8>::new());
}

#[cfg(feature = "alloc")]
#[test]
fn storage_vec_dynamic_string_set_replaces_inline_with_spilled() {
    // Replacing a short (inline) element with a long (spilled) one must
    // overwrite cleanly through `T::write_to_storage`.
    let mut v = unsafe { StorageVec::<String>::new(StorageKey::from_slot(0), h()) };
    v.push(&String::from("short"));
    let long: String = str::repeat("x", 80);
    v.set(0, &long);
    assert_eq!(v.get(0), long);
}

#[cfg(feature = "alloc")]
#[test]
fn storage_vec_dynamic_pop_clears_spilled_body() {
    // pop must call T::clear_storage which tears down both header and any
    // spilled body chunks — verify via len + try_get.
    let mut v = unsafe { StorageVec::<String>::new(StorageKey::from_slot(0), h()) };
    let s100: String = str::repeat("x", 100); // spilled
    let s80: String = str::repeat("y", 80); // spilled
    v.push(&s100);
    v.push(&s80);

    let popped: Option<String> = v.pop();
    assert_eq!(popped.as_ref().map(|s| s.len()), Some(80usize));
    assert_eq!(v.len(), 1);
    assert_eq!(v.try_get(1), None);
    assert_eq!(v.get(0).len(), 100);
}

#[cfg(feature = "alloc")]
#[test]
fn storage_vec_dynamic_clear_zeros_spilled_bodies() {
    // clear() must call T::clear_storage on every element, tearing down
    // both the inline header and any spilled body chunks. Verify the
    // header slots AND a representative spilled-body slot all read zero
    // after clear().
    let host = h();
    let mut v = unsafe { StorageVec::<String>::new(StorageKey::from_slot(0), host.clone()) };
    let s_a: String = str::repeat("a", 100); // spilled (4 body chunks)
    let s_b: String = str::repeat("b", 80); // spilled (3 body chunks)
    let s_c: String = str::repeat("c", 50); // spilled (2 body chunks)
    v.push(&s_a);
    v.push(&s_b);
    v.push(&s_c);
    assert_eq!(v.len(), 3);

    // Capture the first spilled-body chunk of element 0 (proves it's
    // non-zero before clear, so the post-clear assertion is meaningful).
    let body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let header_e0 = body;
    let mut spilled_e0_chunk0 = [0u8; 32];
    host.hash_keccak_256(&header_e0, &mut spilled_e0_chunk0);
    assert_ne!(storage_get_32(&host, &spilled_e0_chunk0), [0u8; 32]);

    v.clear();
    assert_eq!(v.len(), 0);
    assert!(v.is_empty());

    // Outer length slot is zero.
    assert_eq!(
        storage_get_32(&host, StorageKey::from_slot(0).as_bytes()),
        [0u8; 32]
    );

    // Each element's header slot (in the array body) is zero.
    let mut header_key = body;
    for _ in 0..3 {
        assert_eq!(storage_get_32(&host, &header_key), [0u8; 32]);
        inc_slot(&mut header_key);
    }

    // The previously-non-zero spilled chunk of element 0 is now zero.
    assert_eq!(storage_get_32(&host, &spilled_e0_chunk0), [0u8; 32]);
}

// --- Mapping<K, StorageVec<T>> — `mapping(K => T[])` in Solidity ---

#[test]
fn mapping_of_storage_vec_roundtrip() {
    let mut m = unsafe { Mapping::<Address, StorageVec<U256>>::new(StorageKey::from_slot(4), h()) };
    let alice = Address([0xAA; 20]);
    let bob = Address([0xBB; 20]);

    {
        let mut alice_vec = m.entry(&alice);
        alice_vec.push(&U256::from(1u64));
        alice_vec.push(&U256::from(2u64));
        alice_vec.push(&U256::from(3u64));
    }
    {
        let mut bob_vec = m.entry(&bob);
        bob_vec.push(&U256::from(99u64));
    }

    assert_eq!(m.get(&alice).len(), 3);
    assert_eq!(m.get(&alice).get(0), U256::from(1u64));
    assert_eq!(m.get(&alice).get(2), U256::from(3u64));
    assert_eq!(m.get(&bob).len(), 1);
    assert_eq!(m.get(&bob).get(0), U256::from(99u64));
}

#[test]
fn mapping_of_storage_vec_keys_are_independent() {
    // Two keys must produce non-overlapping element ranges.
    let mut m = unsafe { Mapping::<Address, StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };
    let a = Address([0x11; 20]);
    let b = Address([0x22; 20]);

    m.entry(&a).push(&U256::from(100u64));
    m.entry(&b).push(&U256::from(200u64));
    m.entry(&a).push(&U256::from(101u64));

    assert_eq!(m.get(&a).len(), 2);
    assert_eq!(m.get(&b).len(), 1);
    assert_eq!(m.get(&a).get(0), U256::from(100u64));
    assert_eq!(m.get(&a).get(1), U256::from(101u64));
    assert_eq!(m.get(&b).get(0), U256::from(200u64));
}

#[test]
fn mapping_of_storage_vec_matches_solc_layout() {
    // Cross-check that `Mapping<K, StorageVec<T>>` lays out elements at the
    // same slots solc would for `mapping(K => T[]) at slot S`:
    //   T[]_slot       = keccak256(pad32(K) ++ pad32(S))
    //   length         = T[]_slot
    //   element i      = keccak256(T[]_slot) + i
    let host = h();
    let mut m = unsafe {
        Mapping::<Address, StorageVec<U256>>::new(StorageKey::from_slot(7), host.clone())
    };
    let key = Address([0x33; 20]);

    m.entry(&key).push(&U256::from(0xDEADu64));
    m.entry(&key).push(&U256::from(0xBEEFu64));

    // Derive the inner-vec root the same way `Mapping::slot_of` does.
    let inner_root = StorageKey::from_slot(7).derive(&host, &key);
    // Length lives at the inner root.
    let len_slot = storage_get_32(&host, inner_root.as_bytes());
    assert_eq!(len_slot[31], 2);

    // Element 0 lives at keccak256(inner_root).
    let body_base = storage_derive_body_base(&host, inner_root.as_bytes());
    let e0 = storage_get_32(&host, &body_base);
    assert_eq!(U256::from_be_bytes(e0), U256::from(0xDEADu64));

    let mut e1_key = body_base;
    inc_slot(&mut e1_key);
    let e1 = storage_get_32(&host, &e1_key);
    assert_eq!(U256::from_be_bytes(e1), U256::from(0xBEEFu64));
}

#[test]
fn mapping_of_storage_vec_view_borrow_is_read_only() {
    // Behavioural check that the `Ref<'_, StorageVec<T>>` returned by
    // `Mapping<K, StorageVec<T>>::get` exposes only `&self` methods. The
    // compile-fail UI test pins the type-level guarantee; this test
    // exercises the read path.
    let mut m = unsafe { Mapping::<Address, StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };
    let k = Address([0x44; 20]);
    m.entry(&k).push(&U256::from(7u64));

    let view = m.get(&k);
    assert_eq!(view.len(), 1);
    assert_eq!(view.get(0), U256::from(7u64));
    assert_eq!(view.try_get(1), None);
}

// --- StorageVec<StorageVec<T>> — `T[][]` in Solidity ---

#[test]
fn nested_storage_vec_roundtrip() {
    let mut outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };

    outer.grow();
    outer.grow();
    assert_eq!(outer.len(), 2);

    {
        let mut row0 = outer.entry(0);
        row0.push(&U256::from(10u64));
        row0.push(&U256::from(20u64));
    }
    {
        let mut row1 = outer.entry(1);
        row1.push(&U256::from(30u64));
    }

    assert_eq!(outer.get(0).len(), 2);
    assert_eq!(outer.get(0).get(0), U256::from(10u64));
    assert_eq!(outer.get(0).get(1), U256::from(20u64));
    assert_eq!(outer.get(1).len(), 1);
    assert_eq!(outer.get(1).get(0), U256::from(30u64));
}

#[test]
fn nested_storage_vec_grow_populates_in_one_step() {
    let mut outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };

    {
        let mut row = outer.grow();
        row.push(&U256::from(1u64));
        row.push(&U256::from(2u64));
    }
    {
        let mut row = outer.grow();
        row.push(&U256::from(3u64));
    }

    assert_eq!(outer.len(), 2);
    assert_eq!(outer.get(0).len(), 2);
    assert_eq!(outer.get(0).get(0), U256::from(1u64));
    assert_eq!(outer.get(0).get(1), U256::from(2u64));
    assert_eq!(outer.get(1).len(), 1);
    assert_eq!(outer.get(1).get(0), U256::from(3u64));

    // grow() targets the same inner root that grow() + entry would.
    // Use one shared host so both write paths hit the same backing store.
    let host = h();
    {
        let mut via_steps =
            unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(5), host.clone()) };
        via_steps.grow();
        via_steps.entry(0).push(&U256::from(42u64));
    }
    let mut via_grow =
        unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(5), host) };
    // A fresh handle at the same slot reads back what grow() + entry wrote.
    assert_eq!(via_grow.get(0).get(0), U256::from(42u64));
    via_grow.grow().push(&U256::from(7u64));
    assert_eq!(via_grow.get(1).get(0), U256::from(7u64));
}

#[test]
fn nested_storage_vec_inner_rows_are_independent() {
    // Two different outer indices must derive non-overlapping inner roots.
    let mut outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };

    outer.grow();
    outer.grow();
    outer.entry(0).push(&U256::from(0xAAu64));
    outer.entry(1).push(&U256::from(0xBBu64));
    outer.entry(0).push(&U256::from(0xCCu64));

    assert_eq!(outer.get(0).len(), 2);
    assert_eq!(outer.get(1).len(), 1);
    assert_eq!(outer.get(0).get(0), U256::from(0xAAu64));
    assert_eq!(outer.get(0).get(1), U256::from(0xCCu64));
    assert_eq!(outer.get(1).get(0), U256::from(0xBBu64));
}

#[test]
fn nested_storage_vec_matches_solc_layout() {
    // Cross-check that nested layout matches solc's `T[][] at slot S`:
    //   inner_root(i) = keccak256(pad32(S)) + i
    //   inner length  at inner_root(i)
    //   inner body    at keccak256(inner_root(i)) + j
    let host = h();
    let mut outer =
        unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(11), host.clone()) };

    outer.grow();
    outer.grow();
    outer.grow();
    outer.entry(0).push(&U256::from(0x1111u64));
    outer.entry(2).push(&U256::from(0x2222u64));
    outer.entry(2).push(&U256::from(0x3333u64));

    let outer_body = storage_derive_body_base(&host, StorageKey::from_slot(11).as_bytes());

    // inner_root(0) = outer_body + 0
    let inner0_root = outer_body;
    let inner0_len = storage_get_32(&host, &inner0_root);
    assert_eq!(inner0_len[31], 1, "inner[0] length");

    let inner0_body = storage_derive_body_base(&host, &inner0_root);
    let inner0_e0 = storage_get_32(&host, &inner0_body);
    assert_eq!(U256::from_be_bytes(inner0_e0), U256::from(0x1111u64));

    // inner_root(2) = outer_body + 2
    let mut inner2_root = outer_body;
    inc_slot_by(&mut inner2_root, 2);
    let inner2_len = storage_get_32(&host, &inner2_root);
    assert_eq!(inner2_len[31], 2, "inner[2] length");

    let inner2_body = storage_derive_body_base(&host, &inner2_root);
    let inner2_e0 = storage_get_32(&host, &inner2_body);
    assert_eq!(U256::from_be_bytes(inner2_e0), U256::from(0x2222u64));

    let mut inner2_e1_key = inner2_body;
    inc_slot(&mut inner2_e1_key);
    let inner2_e1 = storage_get_32(&host, &inner2_e1_key);
    assert_eq!(U256::from_be_bytes(inner2_e1), U256::from(0x3333u64));
}

#[test]
fn nested_storage_vec_len_tracks_grow() {
    let mut outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };

    assert_eq!(outer.len(), 0);
    assert!(outer.is_empty());

    outer.grow();
    assert_eq!(outer.len(), 1);
    assert!(!outer.is_empty());

    outer.grow();
    outer.grow();
    assert_eq!(outer.len(), 3);

    // Inner array at index 0 is empty (no elements pushed).
    assert_eq!(outer.get(0).len(), 0);
    assert!(outer.get(0).is_empty());
}

#[test]
fn nested_storage_vec_view_borrow_is_read_only() {
    let mut outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };
    outer.grow();
    outer.entry(0).push(&U256::from(42u64));

    let view = outer.get(0);
    assert_eq!(view.len(), 1);
    assert_eq!(view.get(0), U256::from(42u64));
    assert_eq!(view.try_get(1), None);
}

#[test]
fn nested_storage_vec_storage_component_metadata() {
    // `StorageVec<StorageVec<T>>` plugs into the `#[storage]` /
    // `#[contract]` macro path as a full-slot, non-packing component —
    // matches the flat `StorageVec<T>` shape exactly.
    assert_eq!(<StorageVec<StorageVec<U256>> as StorageComponent>::SLOTS, 1);
    assert_eq!(
        <StorageVec<StorageVec<U256>> as StorageComponent>::PACKED_BYTES,
        32
    );
}

#[test]
fn nested_storage_vec_new_at_matches_new() {
    // `StorageComponent::new_at` is the macro-emitted safe path; assert it
    // produces a handle indistinguishable from `unsafe { new(...) }`.
    let host = h();
    let mut a =
        unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(4), host.clone()) };
    let mut b = <StorageVec<StorageVec<U256>> as StorageComponent>::new_at(4, 0, host);
    a.grow();
    a.entry(0).push(&U256::from(0xAAu64));
    // Both handles see the same underlying storage.
    assert_eq!(b.len(), 1);
    assert_eq!(b.get(0).get(0), U256::from(0xAAu64));
    b.entry(0).push(&U256::from(0xBBu64));
    assert_eq!(a.get(0).len(), 2);
    assert_eq!(a.get(0).get(1), U256::from(0xBBu64));
}

#[test]
fn nested_storage_vec_subword_inner_roundtrip() {
    // u64 has PACKED_BYTES = 8, so 4 elements pack per slot in the inner
    // body. Verify nested dispatch threads through to sub-word read/write
    // and that two inner rows packing 6 and 10 elements stay independent.
    let mut outer = unsafe { StorageVec::<StorageVec<u64>>::new(StorageKey::from_slot(0), h()) };
    outer.grow();
    outer.grow();

    {
        let mut row0 = outer.entry(0);
        for i in 0u64..6 {
            row0.push(&(0xA000 + i));
        }
    }
    {
        let mut row1 = outer.entry(1);
        for i in 0u64..10 {
            row1.push(&(0xB000 + i));
        }
    }

    let r0 = outer.get(0);
    assert_eq!(r0.len(), 6);
    for i in 0u64..6 {
        assert_eq!(r0.get(i), 0xA000 + i);
    }
    drop(r0);
    let r1 = outer.get(1);
    assert_eq!(r1.len(), 10);
    for i in 0u64..10 {
        assert_eq!(r1.get(i), 0xB000 + i);
    }
}

#[test]
fn nested_storage_vec_erase_last_recursively_clears_inner() {
    // erase_last() must destroy the removed inner array's storage — both its
    // length slot and its body slots — matching solc `T[][].pop()`.
    let host = h();
    let mut outer =
        unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), host.clone()) };
    outer.grow();
    outer.grow();
    outer.entry(0).push(&U256::from(11u64));
    outer.entry(1).push(&U256::from(22u64));
    outer.entry(1).push(&U256::from(33u64));

    // Snapshot the soon-to-be-popped inner's length and first body slot
    // as non-zero so the post-pop zero check is meaningful.
    let outer_body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let mut inner1_root = outer_body;
    inc_slot_by(&mut inner1_root, 1);
    let inner1_body = storage_derive_body_base(&host, &inner1_root);
    assert_ne!(storage_get_32(&host, &inner1_root), [0u8; 32]);
    assert_ne!(storage_get_32(&host, &inner1_body), [0u8; 32]);

    assert!(outer.erase_last());
    assert_eq!(outer.len(), 1);

    // Popped inner's length slot and body slots are zero.
    assert_eq!(storage_get_32(&host, &inner1_root), [0u8; 32]);
    assert_eq!(storage_get_32(&host, &inner1_body), [0u8; 32]);
    let mut inner1_body_next = inner1_body;
    inc_slot(&mut inner1_body_next);
    assert_eq!(storage_get_32(&host, &inner1_body_next), [0u8; 32]);

    // Surviving inner row is untouched.
    assert_eq!(outer.get(0).len(), 1);
    assert_eq!(outer.get(0).get(0), U256::from(11u64));

    // erase_last draining to empty returns true once more, then false.
    assert!(outer.erase_last());
    assert_eq!(outer.len(), 0);
    assert!(!outer.erase_last());
}

#[test]
fn nested_storage_vec_clear_recursively_clears_all_inners() {
    // clear() walks every inner, calls its clear(), and zeroes the outer
    // length — matching solc `delete matrix`.
    let host = h();
    let mut outer =
        unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), host.clone()) };
    outer.grow();
    outer.grow();
    outer.entry(0).push(&U256::from(1u64));
    outer.entry(0).push(&U256::from(2u64));
    outer.entry(1).push(&U256::from(3u64));

    let outer_body = storage_derive_body_base(&host, StorageKey::from_slot(0).as_bytes());
    let inner0_root = outer_body;
    let mut inner1_root = outer_body;
    inc_slot(&mut inner1_root);
    let inner0_body = storage_derive_body_base(&host, &inner0_root);
    let inner1_body = storage_derive_body_base(&host, &inner1_root);

    // Sanity: storage is non-zero before clear.
    assert_ne!(storage_get_32(&host, &inner0_root), [0u8; 32]);
    assert_ne!(storage_get_32(&host, &inner0_body), [0u8; 32]);
    assert_ne!(storage_get_32(&host, &inner1_root), [0u8; 32]);
    assert_ne!(storage_get_32(&host, &inner1_body), [0u8; 32]);

    outer.clear();
    assert_eq!(outer.len(), 0);
    assert!(outer.is_empty());

    // Outer length slot is zero.
    assert_eq!(
        storage_get_32(&host, StorageKey::from_slot(0).as_bytes()),
        [0u8; 32]
    );
    // Each inner's length and body slots are zero.
    assert_eq!(storage_get_32(&host, &inner0_root), [0u8; 32]);
    assert_eq!(storage_get_32(&host, &inner0_body), [0u8; 32]);
    let mut inner0_body_next = inner0_body;
    inc_slot(&mut inner0_body_next);
    assert_eq!(storage_get_32(&host, &inner0_body_next), [0u8; 32]);
    assert_eq!(storage_get_32(&host, &inner1_root), [0u8; 32]);
    assert_eq!(storage_get_32(&host, &inner1_body), [0u8; 32]);
}

#[test]
#[should_panic(expected = "out of bounds")]
fn nested_storage_vec_get_panics_on_oob() {
    let outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };
    // len == 0: any index is OOB.
    let _ = outer.get(0);
}

#[test]
#[should_panic(expected = "out of bounds")]
fn nested_storage_vec_entry_panics_on_oob() {
    let mut outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };
    outer.grow();
    // len == 1: index 1 is OOB.
    let _ = outer.entry(1);
}

#[test]
fn nested_storage_vec_read_surface_matches_flat() {
    // try_get / first / last / iter mirror the flat StorageVec read surface,
    // yielding read-only Ref handles to the inner arrays.
    let mut outer = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };

    // Empty: every non-panicking read is None, iter is empty.
    assert!(outer.try_get(0).is_none());
    assert!(outer.first().is_none());
    assert!(outer.last().is_none());
    assert_eq!(outer.iter().count(), 0);

    outer.grow();
    outer.grow();
    outer.grow();
    outer.entry(0).push(&U256::from(10u64));
    outer.entry(1).push(&U256::from(20u64));
    outer.entry(1).push(&U256::from(21u64));
    outer.entry(2).push(&U256::from(30u64));

    // try_get: in-bounds yields a Ref, out-of-bounds yields None.
    assert_eq!(outer.try_get(0).unwrap().get(0), U256::from(10u64));
    assert!(outer.try_get(3).is_none());

    // first / last address the ends.
    assert_eq!(outer.first().unwrap().get(0), U256::from(10u64));
    assert_eq!(outer.last().unwrap().get(0), U256::from(30u64));

    // iter walks the inner arrays front-to-back; ExactSizeIterator len is the
    // outer length captured at construction.
    let mut it = outer.iter();
    assert_eq!(it.len(), 3);
    let lens: alloc::vec::Vec<u64> = outer.iter().map(|row| row.len()).collect();
    assert_eq!(lens, alloc::vec![1, 2, 1]);
    // First element of each row, via iter.
    let firsts: alloc::vec::Vec<U256> = outer.iter().map(|row| row.get(0)).collect();
    assert_eq!(
        firsts,
        alloc::vec![U256::from(10u64), U256::from(20u64), U256::from(30u64)]
    );

    // DoubleEndedIterator: back-to-front yields rows in reverse.
    let rev_firsts: alloc::vec::Vec<U256> = outer.iter().rev().map(|row| row.get(0)).collect();
    assert_eq!(
        rev_firsts,
        alloc::vec![U256::from(30u64), U256::from(20u64), U256::from(10u64)]
    );

    // Consume the front iterator handle so the borrow ends cleanly.
    let _ = it.next();
}

#[test]
fn inc_slot_by_zero_is_identity() {
    let mut slot = [0u8; 32];
    slot[31] = 0xAB;
    let original = slot;
    inc_slot_by(&mut slot, 0);
    assert_eq!(slot, original);
}

#[test]
fn inc_slot_by_small() {
    let mut slot = [0u8; 32];
    inc_slot_by(&mut slot, 5);
    let mut expected = [0u8; 32];
    expected[31] = 5;
    assert_eq!(slot, expected);
}

#[test]
fn inc_slot_by_carries_through_low_bytes() {
    let mut slot = [0u8; 32];
    slot[31] = 0xFF;
    inc_slot_by(&mut slot, 1);
    let mut expected = [0u8; 32];
    expected[30] = 1;
    expected[31] = 0;
    assert_eq!(slot, expected);
}

#[test]
fn inc_slot_by_full_u64_range() {
    let mut slot = [0u8; 32];
    inc_slot_by(&mut slot, u64::MAX);
    // Lowest 8 bytes should be 0xFF...FF, upper 24 bytes zero.
    let mut expected = [0u8; 32];
    expected[24..].copy_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(slot, expected);
}

#[test]
fn inc_slot_by_carries_into_high_bytes() {
    // Start at "all 0xFF in low 8 bytes", add 1 -> carry into byte 23.
    let mut slot = [0u8; 32];
    slot[24..].fill(0xFF);
    inc_slot_by(&mut slot, 1);
    let mut expected = [0u8; 32];
    expected[23] = 1;
    // Bytes 24..32 wrap to zero.
    assert_eq!(slot, expected);
}

// --- Probing tests added during deep review of nested / grow API ---

#[test]
fn probe_nested_erase_last_then_grow_no_stale_data() {
    let mut m = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), h()) };
    {
        let mut row = m.grow();
        row.push(&U256::from(111u64));
        row.push(&U256::from(222u64));
    }
    assert_eq!(m.get(0).len(), 2);
    // erase the row (clears its body), then add a fresh row at the SAME outer index.
    assert!(m.erase_last());
    assert_eq!(m.len(), 0);
    {
        let mut row = m.grow();
        row.push(&U256::from(999u64));
    }
    // The new row must NOT see the old length(2) or old elements.
    assert_eq!(m.get(0).len(), 1);
    assert_eq!(m.get(0).get(0), U256::from(999u64));
}

#[test]
fn probe_nested_entry_grows_inner_independently() {
    let mut m = unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(3), h()) };
    m.grow();
    m.grow();
    m.entry(0).push(&U256::from(1u64));
    m.entry(0).push(&U256::from(2u64));
    m.entry(1).push(&U256::from(9u64));
    assert_eq!(m.get(0).len(), 2);
    assert_eq!(m.get(1).len(), 1);
    assert_eq!(m.get(0).get(1), U256::from(2u64));
    assert_eq!(m.get(1).get(0), U256::from(9u64));
}
