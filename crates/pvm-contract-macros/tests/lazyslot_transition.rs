//! Stress-test bare `String` fields inside structs used as `Mapping` values:
//! long→short transitions, multiple updates, body cleanup, etc.
extern crate alloc;
use pvm_contract_sdk::{Address, Mapping, SolStorage, SolType, StorageKey};
use pvm_contract_types::MockHostBuilder;

fn h() -> pvm_contract_sdk::Host {
    pvm_contract_sdk::Host::from_dyn(alloc::rc::Rc::new(MockHostBuilder::new().build()))
}

#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct R {
    pub a: Address,
    pub s: alloc::string::String,
}

#[derive(Clone, Debug, PartialEq, Eq, SolType, SolStorage)]
pub struct MultiDyn {
    pub name: alloc::string::String,
    pub bio: alloc::string::String,
    pub tag: u32,
}

#[test]
fn struct_with_multiple_dynamic_fields_round_trip() {
    let host = h();
    let mut m = unsafe { Mapping::<u64, MultiDyn>::new(StorageKey::from_slot(0), host) };
    let v = MultiDyn {
        name: "alice".to_string(),
        bio: "y".repeat(80), // long, spills to body
        tag: 42,
    };
    m.insert(&9, &v);
    let got = m.get(&9);
    assert_eq!(got, v);
}

#[test]
fn struct_with_multiple_dynamic_fields_overwrite_clears_bodies() {
    use pvm_contract_sdk::{HostApi, StorageFlags};
    let host = h();
    let mut m = unsafe { Mapping::<u64, MultiDyn>::new(StorageKey::from_slot(0), host.clone()) };
    m.insert(
        &9,
        &MultiDyn {
            name: "x".repeat(80),
            bio: "y".repeat(80),
            tag: 0,
        },
    );
    // Overwrite with short values.
    m.insert(
        &9,
        &MultiDyn {
            name: "x".to_string(),
            bio: "y".to_string(),
            tag: 99,
        },
    );
    let got = m.get(&9);
    assert_eq!(got.name, "x");
    assert_eq!(got.bio, "y");
    assert_eq!(got.tag, 99);

    // Verify both fields' body chunks are cleared.
    let base = m.slot_of(&9);
    for slot_offset in 0..2u64 {
        let mut field_slot = *base.as_bytes();
        for _ in 0..slot_offset {
            for byte in field_slot.iter_mut().rev() {
                let (n, c) = byte.overflowing_add(1);
                *byte = n;
                if !c {
                    break;
                }
            }
        }
        let mut body0 = [0u8; 32];
        host.hash_keccak_256(&field_slot, &mut body0);
        let mut p = [0u8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), &body0, &mut p);
        assert_eq!(p, [0u8; 32], "field {slot_offset} body chunk 0 not cleared");
    }
}

#[test]
fn try_get_recognizes_dynamic_only_set() {
    let host = h();
    let mut m = unsafe { Mapping::<u64, R>::new(StorageKey::from_slot(0), host) };
    m.insert(
        &7,
        &R {
            a: Address([0; 20]),
            s: "hello".to_string(),
        },
    );
    let got = m.try_get(&7);
    assert!(
        got.is_some(),
        "try_get should see entry whose only non-zero slot is the dynamic field"
    );
    assert_eq!(got.unwrap().s, "hello");
}

#[test]
fn remove_must_clear_dynamic_body_chunks() {
    use pvm_contract_sdk::{HostApi, StorageFlags};
    let host = h();
    let mut m = unsafe { Mapping::<u64, R>::new(StorageKey::from_slot(0), host.clone()) };
    let long = "z".repeat(80);
    m.insert(
        &3,
        &R {
            a: Address([0xaa; 20]),
            s: long,
        },
    );

    let base = m.slot_of(&3);
    let mut s_slot = *base.as_bytes();
    for byte in s_slot.iter_mut().rev() {
        let (n, c) = byte.overflowing_add(1);
        *byte = n;
        if !c {
            break;
        }
    }
    let mut body0 = [0u8; 32];
    host.hash_keccak_256(&s_slot, &mut body0);

    m.remove(&3);

    let mut probe = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), &s_slot, &mut probe);
    assert_eq!(probe, [0u8; 32], "header not cleared after remove");

    let mut chunk = body0;
    for i in 0..3 {
        let mut p = [0u8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), &chunk, &mut p);
        assert_eq!(p, [0u8; 32], "body chunk {i} not cleared after remove");
        for byte in chunk.iter_mut().rev() {
            let (n, c) = byte.overflowing_add(1);
            *byte = n;
            if !c {
                break;
            }
        }
    }
}

#[test]
fn long_then_short_must_clear_stale_body_chunks() {
    use pvm_contract_sdk::{HostApi, StorageFlags};
    let host = h();
    let mut m = unsafe { Mapping::<u64, R>::new(StorageKey::from_slot(0), host.clone()) };
    // Long value: 80 bytes → 3 body chunks at keccak256(s_slot)+i.
    let long = "x".repeat(80);
    m.insert(
        &1,
        &R {
            a: Address([0; 20]),
            s: long,
        },
    );

    // The dynamic field lives at base + 1 (reviewer at base + 0).
    let base = m.slot_of(&1);
    let mut s_slot = *base.as_bytes();
    for byte in s_slot.iter_mut().rev() {
        let (n, c) = byte.overflowing_add(1);
        *byte = n;
        if !c {
            break;
        }
    }

    let mut body0 = [0u8; 32];
    host.hash_keccak_256(&s_slot, &mut body0);
    let mut probe = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), &body0, &mut probe);
    assert_ne!(
        probe, [0u8; 32],
        "long write should have populated body chunk 0"
    );

    m.insert(
        &1,
        &R {
            a: Address([0; 20]),
            s: "y".repeat(10),
        },
    );

    let mut chunk_slot = body0;
    for i in 0..3 {
        let mut probe = [0u8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), &chunk_slot, &mut probe);
        assert_eq!(
            probe, [0u8; 32],
            "body chunk {i} not cleared after long→short transition"
        );
        for byte in chunk_slot.iter_mut().rev() {
            let (n, c) = byte.overflowing_add(1);
            *byte = n;
            if !c {
                break;
            }
        }
    }

    let got = m.get(&1);
    assert_eq!(got.s, "y".repeat(10));
}
