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

use core::marker::PhantomData;
use pallet_revive_uapi::{HostFn, HostFnImpl as api, StorageFlags};
use parity_scale_codec::{Decode, Encode};

/// Default buffer size for reading values from storage.
const DEFAULT_READ_BUFFER_SIZE: usize = 512;

/// A 32-byte storage key.
pub type StorageKey = [u8; 32];

// ============================================================================
// Key Hashing
// ============================================================================

/// Hash any SCALE-encodable data into a 32-byte storage key using Keccak256.
pub fn hash_key<T: Encode>(data: &T) -> StorageKey {
    let encoded = data.encode();
    let mut key = [0u8; 32];
    api::hash_keccak_256(&encoded, &mut key);
    key
}

/// Combine a namespace with a key to create a namespaced storage key.
pub fn namespaced_key<K: Encode>(namespace: &[u8], key: &K) -> StorageKey {
    let mut data = namespace.encode();
    key.encode_to(&mut data);

    let mut result = [0u8; 32];
    api::hash_keccak_256(&data, &mut result);
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
pub fn get<T: Decode>(key: &StorageKey) -> Option<T> {
    get_with_buffer::<T, DEFAULT_READ_BUFFER_SIZE>(key)
}

/// Get a value from storage with a custom buffer size.
pub fn get_with_buffer<T: Decode, const N: usize>(key: &StorageKey) -> Option<T> {
    let mut buffer = [0u8; N];
    let mut output = buffer.as_mut_slice();

    match api::get_storage(StorageFlags::empty(), key, &mut output) {
        Ok(_) => {
            let bytes_read = N - output.len();
            T::decode(&mut &buffer[..bytes_read]).ok()
        }
        Err(_) => None,
    }
}

/// Set a value in storage at the given key.
pub fn set<T: Encode>(key: &StorageKey, value: &T) {
    let encoded = value.encode();
    api::set_storage(StorageFlags::empty(), key, &encoded);
}

/// Remove a value from storage.
pub fn remove(key: &StorageKey) {
    api::set_storage(StorageFlags::empty(), key, &[]);
}

/// Check if a key exists in storage.
pub fn contains(key: &StorageKey) -> bool {
    let mut buffer = [0u8; 1];
    let mut output = buffer.as_mut_slice();
    api::get_storage(StorageFlags::empty(), key, &mut output).is_ok()
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
        api::hash_keccak_256(&data, &mut result);
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
