extern crate alloc;

use pvm_contract_macros::SolEvent;
use pvm_contract_types::{Address, SolEvent as SolEventTrait};
use ruint::aliases::U256;

#[derive(SolEvent)]
struct Transfer {
    #[indexed]
    from: Address,
    #[indexed]
    to: Address,
    value: U256,
}

#[test]
fn topic0_matches_keccak256_of_signature() {
    let expected = pvm_contract_types::const_event_topic("Transfer(address,address,uint256)");
    assert_eq!(Transfer::TOPIC, expected);
}

#[test]
fn signature_is_canonical() {
    assert_eq!(Transfer::SIGNATURE, "Transfer(address,address,uint256)");
}

#[test]
fn name_is_struct_name() {
    assert_eq!(Transfer::NAME, "Transfer");
}

#[test]
fn indexed_count_is_correct() {
    assert_eq!(Transfer::INDEXED_COUNT, 2);
}

#[test]
fn topics_returns_three_entries() {
    let event = Transfer {
        from: Address([0xAA; 20]),
        to: Address([0xBB; 20]),
        value: U256::from(100u64),
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 3);
}

#[test]
fn topic0_is_signature_hash() {
    let event = Transfer {
        from: Address([0xAA; 20]),
        to: Address([0xBB; 20]),
        value: U256::from(100u64),
    };
    let topics = event.topics();
    assert_eq!(topics[0], Transfer::TOPIC);
}

#[test]
fn indexed_addresses_are_right_aligned() {
    let event = Transfer {
        from: Address([0xAA; 20]),
        to: Address([0xBB; 20]),
        value: U256::from(100u64),
    };
    let topics = event.topics();

    assert_eq!(&topics[1][..12], &[0u8; 12]);
    assert_eq!(&topics[1][12..], &[0xAA; 20]);
    assert_eq!(&topics[2][..12], &[0u8; 12]);
    assert_eq!(&topics[2][12..], &[0xBB; 20]);
}

#[test]
fn data_encodes_non_indexed_value() {
    let event = Transfer {
        from: Address([0xAA; 20]),
        to: Address([0xBB; 20]),
        value: U256::from(42u64),
    };
    let data = event.data();
    assert_eq!(data.len(), 32);
    let decoded = <U256 as pvm_contract_types::SolDecode>::decode(&data);
    assert_eq!(decoded, U256::from(42u64));
}

// Cross-check topic hash against alloy
mod alloy_cross_check {
    use alloy_core::primitives::keccak256;
    use pvm_contract_types::SolEvent as _;

    #[test]
    fn topic0_matches_alloy_keccak256() {
        let sig = "Transfer(address,address,uint256)";
        let alloy_hash = keccak256(sig.as_bytes());
        assert_eq!(super::Transfer::TOPIC, alloy_hash.0);
    }
}

// Event with no indexed fields
#[derive(SolEvent)]
struct Log {
    value: u64,
    flag: bool,
}

#[test]
fn no_indexed_fields_topic_count() {
    let event = Log {
        value: 42,
        flag: true,
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 1, "only topic0 when no indexed fields");
    assert_eq!(topics[0], Log::TOPIC);
}

#[test]
fn no_indexed_fields_data() {
    let event = Log {
        value: 99,
        flag: true,
    };
    let data = event.data();
    assert_eq!(data.len(), 64, "two 32-byte words for u64 + bool");
}

#[test]
fn no_indexed_signature() {
    assert_eq!(Log::SIGNATURE, "Log(uint64,bool)");
    assert_eq!(Log::INDEXED_COUNT, 0);
}

// Event with all indexed fields (no data)
#[derive(SolEvent)]
struct Approval {
    #[indexed]
    owner: Address,
    #[indexed]
    spender: Address,
    #[indexed]
    value: U256,
}

#[test]
fn all_indexed_topic_count() {
    let event = Approval {
        owner: Address([1; 20]),
        spender: Address([2; 20]),
        value: U256::from(500u64),
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 4, "topic0 + 3 indexed");
}

#[test]
fn all_indexed_empty_data() {
    let event = Approval {
        owner: Address([1; 20]),
        spender: Address([2; 20]),
        value: U256::from(500u64),
    };
    let data = event.data();
    assert_eq!(data.len(), 0, "no data when all fields indexed");
}

#[test]
fn all_indexed_signature() {
    assert_eq!(Approval::SIGNATURE, "Approval(address,address,uint256)");
    assert_eq!(Approval::INDEXED_COUNT, 3);
}

// Verify U256 indexed topic is packed as 32 big-endian bytes
#[test]
fn u256_indexed_topic_packing() {
    let event = Approval {
        owner: Address([0; 20]),
        spender: Address([0; 20]),
        value: U256::from(0xDEADBEEFu64),
    };
    let topics = event.topics();
    let value_topic = topics[3];
    // U256 big-endian: last 4 bytes should be 0xDEADBEEF
    assert_eq!(value_topic[28], 0xDE);
    assert_eq!(value_topic[29], 0xAD);
    assert_eq!(value_topic[30], 0xBE);
    assert_eq!(value_topic[31], 0xEF);
    assert_eq!(&value_topic[..28], &[0u8; 28]);
}
