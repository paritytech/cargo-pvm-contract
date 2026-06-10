//! Storage abstraction layer for PolkaVM smart contracts.
//!
//! Provides type-safe storage operations with automatic SCALE encoding/decoding
//! and key hashing.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use pvm_contract::storage::{hash_key, get, set};
//!
//! // Create a key by hashing data
//! let key = hash_key(&("balances", user_address));
//!
//! // Store a value
//! set(&key, &1000u128);
//!
//! // Retrieve a value
//! let balance: Option<u128> = get(&key);
//! ```
//!
//! # Using the `#[storage]` macro
//!
//! ```rust,ignore
//! #[pvm_contract::storage]
//! struct Storage {
//!     owner: Address,
//!     total_supply: U256,
//!     balances: Mapping<Address, U256>,
//! }
//!
//! // Access storage via generated methods:
//! Storage::owner().set(&caller);
//! Storage::total_supply().get();
//! Storage::balances().insert(&addr, &amount);
//! ```

mod ordered_index;

pub use ordered_index::OrderedIndex;

use core::marker::PhantomData;
use parity_scale_codec::{Decode, Encode};

// ============================================================================
// Raw storage backend
// ============================================================================
//
// All raw host interactions (storage get/set, keccak hashing, revert) go
// through the free functions in `backend`. On the PolkaVM target this is a
// set of `#[inline(always)]` wrappers over `pallet_revive_uapi` — zero-cost,
// compiling to exactly the same code as calling the host functions directly.
// On the host (only ever compiled under `cfg(test)` or the `host-test`
// feature) the same functions are backed by a thread-local `HashMap`, which
// lets the storage primitives — most importantly `OrderedIndex` — be tested
// with plain `cargo test`. The host shim mirrors the on-chain semantics this
// module relies on: `remove` writes an empty value (a 0-byte row that
// `get_storage` still reports as `Ok`), and reads truncate to the caller's
// buffer, shrinking the output slice to the bytes written.

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
mod backend {
    use pallet_revive_uapi::{HostFn, HostFnImpl as api, ReturnFlags, StorageFlags};

    #[inline(always)]
    pub fn storage_get(key: &[u8; 32], output: &mut &mut [u8]) -> Result<(), ()> {
        api::get_storage(StorageFlags::empty(), key, output).map_err(|_| ())
    }

    #[inline(always)]
    pub fn storage_set(key: &[u8; 32], value: &[u8]) {
        api::set_storage(StorageFlags::empty(), key, value);
    }

    #[inline(always)]
    pub fn hash_keccak_256(input: &[u8], output: &mut [u8; 32]) {
        api::hash_keccak_256(input, output);
    }

    #[inline(always)]
    pub fn revert(msg: &[u8]) -> ! {
        api::return_value(ReturnFlags::REVERT, msg)
    }
}

#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
mod backend {
    //! Host-side shim: a thread-local key-value map standing in for the
    //! revive child trie. Thread-locality gives each test thread an isolated
    //! storage universe.
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::vec::Vec;

    std::thread_local! {
        static STORAGE: RefCell<HashMap<[u8; 32], Vec<u8>>> = RefCell::new(HashMap::new());
        static READS: RefCell<u64> = const { RefCell::new(0) };
    }

    pub fn storage_get(key: &[u8; 32], output: &mut &mut [u8]) -> Result<(), ()> {
        READS.with(|r| *r.borrow_mut() += 1);
        STORAGE.with(|s| match s.borrow().get(key) {
            Some(value) => {
                let n = core::cmp::min(value.len(), output.len());
                output[..n].copy_from_slice(&value[..n]);
                // Shrink the output slice to the written length, mirroring
                // `pallet_revive_uapi::get_storage`.
                let out = core::mem::take(output);
                *output = &mut out[..n];
                Ok(())
            }
            None => Err(()),
        })
    }

    pub fn storage_set(key: &[u8; 32], value: &[u8]) {
        STORAGE.with(|s| {
            s.borrow_mut().insert(*key, value.to_vec());
        });
    }

    pub fn hash_keccak_256(input: &[u8], output: &mut [u8; 32]) {
        use tiny_keccak::{Hasher, Keccak};
        let mut keccak = Keccak::v256();
        keccak.update(input);
        keccak.finalize(output);
    }

    /// Host stand-in for an on-chain revert: panic with the revert message so
    /// tests can observe (or `#[should_panic]` on) the named failure.
    pub fn revert(msg: &[u8]) -> ! {
        panic!(
            "contract revert: {}",
            core::str::from_utf8(msg).unwrap_or("<non-utf8 revert message>")
        );
    }

    /// Wipe the thread-local storage map (test isolation helper).
    pub fn reset() {
        STORAGE.with(|s| s.borrow_mut().clear());
        READS.with(|r| *r.borrow_mut() = 0);
    }

    /// Total `storage_get` calls on this thread (test instrumentation).
    pub fn read_count() -> u64 {
        READS.with(|r| *r.borrow())
    }
}

/// Clear the host-side storage shim. Only available off-target; intended for
/// test setup when several storage fixtures share one thread.
#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
pub fn host_storage_reset() {
    backend::reset();
}

/// Total storage reads on this thread since the last reset. Only available
/// off-target; intended for asserting asymptotic read costs in tests.
#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
pub fn host_storage_read_count() -> u64 {
    backend::read_count()
}

/// Default buffer size for reading values from storage.
const DEFAULT_READ_BUFFER_SIZE: usize = 512;

/// Maximum byte length of one durable storage value in the target revive runtime.
pub const MAX_STORAGE_VALUE_BYTES: usize = 416;

/// A 32-byte storage key.
pub type StorageKey = [u8; 32];

// ============================================================================
// Key Hashing
// ============================================================================

/// Hash any SCALE-encodable data into a 32-byte storage key using Keccak256.
pub fn hash_key<T: Encode>(data: &T) -> StorageKey {
    let encoded = data.encode();
    let mut key = [0u8; 32];
    backend::hash_keccak_256(&encoded, &mut key);
    key
}

/// Combine a namespace with a key to create a namespaced storage key.
pub fn namespaced_key<K: Encode>(namespace: &[u8], key: &K) -> StorageKey {
    let mut data = namespace.encode();
    key.encode_to(&mut data);

    let mut result = [0u8; 32];
    backend::hash_keccak_256(&data, &mut result);
    result
}

/// Use a raw 32-byte array as a storage key (no hashing).
pub const fn raw_key(bytes: [u8; 32]) -> StorageKey {
    bytes
}

// ============================================================================
// Value Operations
// ============================================================================

/// Get a value from storage with the given key.
///
/// Returns `None` after a prior [`remove`] of the same key. Today this works
/// indirectly: `remove` calls `api::set_storage(.., &[])`, which the host
/// stores as a 0-byte trie row rather than deleting it (see `remove` for
/// context), so `api::get_storage` returns `Ok` with `output.len() == 0`
/// and `T::decode(&[])` then fails for any non-trivial `T` — yielding
/// `None`. Once the host treats empty-value writes as deletes,
/// `api::get_storage` will return `Err(KeyNotFound)` and the `Err` arm
/// will produce `None` directly. Same result either way.
pub fn get<T: Decode>(key: &StorageKey) -> Option<T> {
    get_with_buffer::<T, DEFAULT_READ_BUFFER_SIZE>(key)
}

/// Get a value from storage with a custom buffer size.
pub fn get_with_buffer<T: Decode, const N: usize>(key: &StorageKey) -> Option<T> {
    let mut buffer = [0u8; N];
    let mut output = buffer.as_mut_slice();

    match backend::storage_get(key, &mut output) {
        Ok(_) => {
            let bytes_read = output.len();
            let _ = output;
            T::decode(&mut &buffer[..bytes_read]).ok()
        }
        Err(_) => None,
    }
}

/// Set a value in storage at the given key.
pub fn set<T: Encode>(key: &StorageKey, value: &T) {
    let encoded = value.encode();
    ensure_storage_value_size(encoded.len());
    backend::storage_set(key, &encoded);
}

/// Remove a value from storage.
///
/// Uses `set_storage` with an empty value rather than `set_storage_or_clear`,
/// because the former operates on the variable-length storage area that
/// `set`/`get` use, while the latter targets the Ethereum-style fixed 32-byte
/// SSTORE area. Mixing them silently leaves data behind.
pub fn remove(key: &StorageKey) {
    backend::storage_set(key, &[]);
}

fn ensure_storage_value_size(len: usize) {
    if len > MAX_STORAGE_VALUE_BYTES {
        revert(b"StorageValueTooLarge")
    }
}

fn revert(msg: &[u8]) -> ! {
    backend::revert(msg)
}

/// Check if a key has a value stored at it.
///
/// Returns `false` after a prior [`remove`] of the same key. Today this
/// matters because `remove` calls `api::set_storage(.., &[])`, which the
/// host stores as a 0-byte trie row rather than deleting it (see `remove`
/// for context). The host's `get_storage` then returns `Ok` for that row
/// even though it has no content — so checking `is_ok()` alone is not
/// enough; we also require `output` to be non-empty. Once the host treats
/// empty-value writes as deletes, the `Ok` + empty-output case becomes
/// unreachable; the check is then structurally redundant but stays correct.
pub fn contains(key: &StorageKey) -> bool {
    let mut buffer = [0u8; 1];
    let mut output = buffer.as_mut_slice();
    match backend::storage_get(key, &mut output) {
        Ok(()) => !output.is_empty(),
        Err(_) => false,
    }
}

// ============================================================================
// Lazy<V> - Single Value Storage
// ============================================================================

/// A single value stored at a fixed storage key.
///
/// "Lazy" because values are not cached - each access goes directly to storage.
///
/// # Example
/// ```rust,ignore
/// let counter: Lazy<u64> = Lazy::new(b"counter");
/// counter.set(&42);
/// let val = counter.get();  // Option<u64>
/// ```
pub struct Lazy<V> {
    key: StorageKey,
    _marker: PhantomData<V>,
}

impl<V> Lazy<V> {
    /// Create a new Lazy storage item with the given namespace.
    pub fn new(namespace: &[u8]) -> Self {
        Self {
            key: hash_key(&namespace),
            _marker: PhantomData,
        }
    }

    /// Create a new Lazy storage item with a raw pre-computed key.
    pub fn from_key(key: StorageKey) -> Self {
        Self {
            key,
            _marker: PhantomData,
        }
    }

    /// Remove the value from storage.
    pub fn clear(&self) {
        remove(&self.key)
    }

    /// Check if a value exists at this key.
    pub fn exists(&self) -> bool {
        contains(&self.key)
    }
}

impl<V: Decode> Lazy<V> {
    /// Get the value from storage.
    pub fn get(&self) -> Option<V> {
        get(&self.key)
    }
}

impl<V: Encode> Lazy<V> {
    /// Set the value in storage.
    pub fn set(&self, value: &V) {
        set(&self.key, value)
    }
}

// ============================================================================
// Mapping<K, V> - Key-Value Storage
// ============================================================================

/// A mapping from keys to values, similar to a HashMap but backed by contract storage.
///
/// Each entry is stored at `hash(namespace + key)`.
///
/// # Example
/// ```rust,ignore
/// let balances: Mapping<Address, u128> = Mapping::new(b"balances");
/// balances.insert(&addr, &1000u128);
/// let bal = balances.get(&addr);  // Option<u128>
/// ```
pub struct Mapping<K, V> {
    namespace: StorageKey,
    _marker: PhantomData<(K, V)>,
}

impl<K, V> Mapping<K, V> {
    /// Create a new Mapping with the given namespace.
    pub fn new(namespace: &[u8]) -> Self {
        Self {
            namespace: hash_key(&namespace),
            _marker: PhantomData,
        }
    }

    /// Create a new Mapping with a raw pre-computed namespace key.
    pub fn from_key(namespace: StorageKey) -> Self {
        Self {
            namespace,
            _marker: PhantomData,
        }
    }
}

impl<K: Encode, V> Mapping<K, V> {
    /// Compute the storage key for a given map key.
    fn storage_key(&self, key: &K) -> StorageKey {
        let mut data = self.namespace.encode();
        key.encode_to(&mut data);

        let mut result = [0u8; 32];
        backend::hash_keccak_256(&data, &mut result);
        result
    }

    /// Remove the value at the given key.
    pub fn remove(&self, key: &K) {
        remove(&self.storage_key(key))
    }

    /// Check if a value exists at the given key.
    pub fn contains(&self, key: &K) -> bool {
        contains(&self.storage_key(key))
    }
}

impl<K: Encode, V: Decode> Mapping<K, V> {
    /// Get the value for a key.
    pub fn get(&self, key: &K) -> Option<V> {
        get(&self.storage_key(key))
    }
}

impl<K: Encode, V: Encode> Mapping<K, V> {
    /// Insert a value at the given key.
    pub fn insert(&self, key: &K, value: &V) {
        set(&self.storage_key(key), value)
    }
}
