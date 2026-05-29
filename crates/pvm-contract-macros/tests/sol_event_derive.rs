extern crate alloc;

use pvm_contract_macros::{SolEvent, SolType};
use pvm_contract_types::{Address, SolEncode, SolEvent as SolEventTrait};
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
fn static_event_emit_e2e() {
    use pvm_contract_types::{Host, MockHostBuilder, SolDecode};

    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(::std::rc::Rc::new(mock.clone()));

    let event = Transfer {
        from: Address([0xAA; 20]),
        to: Address([0xBB; 20]),
        value: U256::from(42u64),
    };
    event.emit(&host);

    let events = mock.events();
    assert_eq!(events.len(), 1);

    let (topics, data) = &events[0];
    assert_eq!(topics.len(), 3);
    assert_eq!(topics[0], Transfer::TOPIC);
    assert_eq!(&topics[1][12..], &[0xAA; 20]);
    assert_eq!(&topics[2][12..], &[0xBB; 20]);
    assert_eq!(U256::decode(data), Ok(U256::from(42u64)));
}

#[test]
fn consts_match_expected_values() {
    assert_eq!(Transfer::SIGNATURE, "Transfer(address,address,uint256)");
    assert_eq!(Transfer::NAME, "Transfer");
    assert_eq!(Transfer::INDEXED_COUNT, 2);
    let expected_topic = pvm_contract_types::const_keccak256(b"Transfer(address,address,uint256)");
    assert_eq!(Transfer::TOPIC, expected_topic);
}

#[test]
fn topic0_is_signature_hash() {
    let event = Transfer {
        from: Address([0xAA; 20]),
        to: Address([0xBB; 20]),
        value: U256::from(100u64),
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 3);
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
    let mut data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut data);
    assert_eq!(data.len(), 32);
    let decoded = <U256 as pvm_contract_types::SolDecode>::decode(&data);
    assert_eq!(decoded, Ok(U256::from(42u64)));
}

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
    let mut data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut data);
    assert_eq!(data.len(), 64, "two 32-byte words for u64 + bool");
}

#[test]
fn no_indexed_signature() {
    assert_eq!(Log::SIGNATURE, "Log(uint64,bool)");
    assert_eq!(Log::INDEXED_COUNT, 0);
}

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
    let mut data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut data);
    assert_eq!(data.len(), 0, "no data when all fields indexed");
}

#[test]
fn all_indexed_signature() {
    assert_eq!(Approval::SIGNATURE, "Approval(address,address,uint256)");
    assert_eq!(Approval::INDEXED_COUNT, 3);
}

#[test]
fn u256_indexed_topic_packing() {
    let event = Approval {
        owner: Address([0; 20]),
        spender: Address([0; 20]),
        value: U256::from(0xDEADBEEFu64),
    };
    let topics = event.topics();
    let value_topic = topics[3];
    assert_eq!(value_topic[28], 0xDE);
    assert_eq!(value_topic[29], 0xAD);
    assert_eq!(value_topic[30], 0xBE);
    assert_eq!(value_topic[31], 0xEF);
    assert_eq!(&value_topic[..28], &[0u8; 28]);
}

#[cfg(feature = "abi-gen")]
fn abi_item_to_json(item: &pvm_contract_types::AbiItem) -> alloc::string::String {
    pvm_contract_types::serde_json::to_string(item).unwrap()
}

#[cfg(feature = "abi-gen")]
#[test]
fn abi_entry_matches_expected_shape() {
    assert_eq!(
        abi_item_to_json(&Transfer::abi_item()),
        r#"{"type":"event","name":"Transfer","inputs":[{"name":"from","type":"address","indexed":true},{"name":"to","type":"address","indexed":true},{"name":"value","type":"uint256","indexed":false}],"anonymous":false}"#
    );
}

#[cfg(feature = "abi-gen")]
#[test]
fn abi_entry_no_indexed_fields() {
    assert_eq!(
        abi_item_to_json(&Log::abi_item()),
        r#"{"type":"event","name":"Log","inputs":[{"name":"value","type":"uint64","indexed":false},{"name":"flag","type":"bool","indexed":false}],"anonymous":false}"#
    );
}

#[cfg(feature = "abi-gen")]
#[test]
fn abi_entry_all_indexed() {
    assert_eq!(
        abi_item_to_json(&Approval::abi_item()),
        r#"{"type":"event","name":"Approval","inputs":[{"name":"owner","type":"address","indexed":true},{"name":"spender","type":"address","indexed":true},{"name":"value","type":"uint256","indexed":true}],"anonymous":false}"#
    );
}

#[cfg(feature = "abi-gen")]
#[derive(SolEvent)]
struct PointMoved {
    point: (u64, u64),
    label: U256,
}

#[cfg(feature = "abi-gen")]
#[test]
fn abi_entry_tuple_field_uses_tuple_with_components() {
    assert_eq!(
        abi_item_to_json(&PointMoved::abi_item()),
        r#"{"type":"event","name":"PointMoved","inputs":[{"name":"point","type":"tuple","components":[{"name":"","type":"uint64"},{"name":"","type":"uint64"}],"indexed":false},{"name":"label","type":"uint256","indexed":false}],"anonymous":false}"#
    );
}

#[derive(SolEvent)]
struct Tagged {
    #[indexed]
    tag: alloc::string::String,
    value: U256,
}

#[test]
fn dynamic_indexed_string_topic_is_keccak_of_raw_bytes() {
    use alloc::string::ToString;

    let tag = "hello".to_string();
    let event = Tagged {
        tag: tag.clone(),
        value: U256::from(42u64),
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 2, "topic0 + 1 indexed");

    let expected = alloy_core::primitives::keccak256(tag.as_bytes()).0;
    assert_eq!(
        topics[1], expected,
        "Indexed string topic must be keccak256 of raw UTF-8 bytes (Solidity event spec)"
    );
}

#[derive(SolEvent)]
struct Mixed {
    value: U256,
    name: alloc::string::String,
}

#[test]
fn multi_field_dynamic_data_matches_alloy_tuple_encoding() {
    use alloc::string::ToString;
    use alloy_core::sol_types::SolValue;

    let event = Mixed {
        value: U256::from(0xDEADBEEFu64),
        name: "hello".to_string(),
    };
    let mut our_data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut our_data);

    let alloy_tuple = (
        alloy_core::primitives::U256::from(0xDEADBEEFu64),
        "hello".to_string(),
    );
    let alloy_data = alloy_tuple.abi_encode_sequence();

    assert_eq!(
        our_data, alloy_data,
        "Event data() must match alloy's flat (uint256,string) tuple encoding"
    );
}

#[derive(SolEvent)]
struct BlobTagged {
    #[indexed]
    blob: pvm_contract_types::Bytes,
    value: U256,
}

#[test]
fn dynamic_indexed_bytes_topic_is_keccak_of_raw_bytes() {
    let raw = alloc::vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02];
    let event = BlobTagged {
        blob: pvm_contract_types::Bytes(raw.clone()),
        value: U256::from(1u64),
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 2, "topic0 + 1 indexed");

    let expected = alloy_core::primitives::keccak256(&raw).0;
    assert_eq!(
        topics[1], expected,
        "Indexed bytes topic must be keccak256 of raw bytes (Solidity event spec)"
    );
}

#[derive(SolEvent)]
struct SingleString {
    name: alloc::string::String,
}

#[test]
fn single_dynamic_field_data_matches_alloy_tuple_encoding() {
    use alloc::string::ToString;
    use alloy_core::sol_types::SolValue;

    let event = SingleString {
        name: "single dynamic field".to_string(),
    };
    let mut our_data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut our_data);

    let alloy_tuple = ("single dynamic field".to_string(),);
    let alloy_data = alloy_tuple.abi_encode_sequence();

    assert_eq!(
        our_data, alloy_data,
        "Single-field event data() must match alloy's (string,) tuple encoding"
    );
}

#[derive(SolEvent)]
struct TwoStrings {
    first: alloc::string::String,
    second: alloc::string::String,
}

#[test]
fn two_dynamic_fields_data_matches_alloy_tuple_encoding() {
    use alloc::string::ToString;
    use alloy_core::sol_types::SolValue;

    let event = TwoStrings {
        first: "hello world".to_string(),
        second: "a longer second value that forces multi-word tail".to_string(),
    };
    let mut our_data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut our_data);

    let alloy_tuple = (
        "hello world".to_string(),
        "a longer second value that forces multi-word tail".to_string(),
    );
    let alloy_data = alloy_tuple.abi_encode_sequence();

    assert_eq!(
        our_data, alloy_data,
        "(string,string) event data() must match alloy tuple sequence encoding"
    );
}

#[derive(SolEvent)]
struct StaticThenDynamics {
    count: U256,
    first: alloc::string::String,
    second: alloc::string::String,
}

#[test]
fn static_plus_two_dynamic_fields_matches_alloy() {
    use alloc::string::ToString;
    use alloy_core::sol_types::SolValue;

    let event = StaticThenDynamics {
        count: U256::from(7u64),
        first: "alpha".to_string(),
        second: "beta".to_string(),
    };
    let mut our_data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut our_data);

    let alloy_tuple = (
        alloy_core::primitives::U256::from(7u64),
        "alpha".to_string(),
        "beta".to_string(),
    );
    let alloy_data = alloy_tuple.abi_encode_sequence();

    assert_eq!(
        our_data, alloy_data,
        "(uint256,string,string) event data() must match alloy tuple sequence encoding"
    );
}

#[derive(SolType)]
struct UserId {
    chain: u64,
    addr: Address,
}

#[derive(SolEvent)]
struct UserCreated {
    #[indexed]
    user: UserId,
    block_number: u64,
}

#[test]
fn indexed_custom_struct_topic_is_keccak_of_abi_encoded_value() {
    let user = UserId {
        chain: 1u64,
        addr: Address([0xAA; 20]),
    };
    let event = UserCreated {
        user: UserId {
            chain: 1u64,
            addr: Address([0xAA; 20]),
        },
        block_number: 42,
    };

    let topics = event.topics();
    assert_eq!(topics.len(), 2, "topic0 + 1 indexed");

    let mut encoded = [0u8; <UserId as SolEncode>::HEAD_SIZE];
    <UserId as SolEncode>::encode_to(&user, &mut encoded);
    let expected = alloy_core::primitives::keccak256(encoded).0;
    assert_eq!(
        topics[1], expected,
        "Indexed custom struct must hash keccak256(abi.encode(value))"
    );
}

#[derive(SolEvent)]
struct StaticTupleEv {
    pair: (u64, u64),
    flag: bool,
}

#[test]
fn static_tuple_non_indexed_field_matches_alloy() {
    use alloy_core::sol_types::SolValue;

    let event = StaticTupleEv {
        pair: (1u64, 2u64),
        flag: true,
    };
    let mut our_data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut our_data);

    let alloy_tuple = ((1u64, 2u64), true);
    let alloy_data = alloy_tuple.abi_encode_sequence();

    assert_eq!(
        our_data, alloy_data,
        "((uint64,uint64),bool) event data() must match alloy tuple sequence encoding"
    );
}

mod alloy_decode_roundtrip {
    use crate::Transfer as OurTransfer;
    use alloy_core::primitives::{B256, LogData};
    use alloy_core::sol_types::SolEvent as AlloySolEvent;
    use pvm_contract_types::{Address, SolEvent as _};
    use ruint::aliases::U256;

    alloy_core::sol! {
        event Transfer(address indexed from, address indexed to, uint256 value);
    }

    #[test]
    fn alloy_decode_log_recovers_static_fields() {
        let our_event = OurTransfer {
            from: Address([0xAA; 20]),
            to: Address([0xBB; 20]),
            value: U256::from(1_000_000u64),
        };

        let our_topics: alloc::vec::Vec<B256> =
            our_event.topics().iter().copied().map(B256::from).collect();
        let mut our_data = alloc::vec![0u8; our_event.data_len()];
        our_event.data_to(&mut our_data);

        let log = LogData::new_unchecked(our_topics, our_data.into());
        let decoded = Transfer::decode_log_data(&log)
            .expect("alloy must be able to decode our derive's wire output");

        assert_eq!(decoded.from.0.0, [0xAA; 20]);
        assert_eq!(decoded.to.0.0, [0xBB; 20]);
        assert_eq!(
            decoded.value,
            alloy_core::primitives::U256::from(1_000_000u64)
        );
    }
}

mod alloy_decode_roundtrip_mixed {
    use crate::Tagged as OurTagged;
    use alloy_core::primitives::{B256, LogData};
    use alloy_core::sol_types::SolEvent as AlloySolEvent;
    use pvm_contract_types::SolEvent as _;
    use ruint::aliases::U256;

    alloy_core::sol! {
        event Tagged(string indexed tag, uint256 value);
    }

    #[test]
    fn alloy_decode_recovers_mixed_fields() {
        use alloc::string::ToString;

        let our_event = OurTagged {
            tag: "category-A".to_string(),
            value: U256::from(99u64),
        };

        let our_topics: alloc::vec::Vec<B256> =
            our_event.topics().iter().copied().map(B256::from).collect();
        let mut our_data = alloc::vec![0u8; our_event.data_len()];
        our_event.data_to(&mut our_data);

        let log = LogData::new_unchecked(our_topics, our_data.into());
        let decoded = Tagged::decode_log_data(&log)
            .expect("alloy must decode our derive's wire output for indexed string + uint256");

        let expected_topic_hash = alloy_core::primitives::keccak256("category-A".as_bytes());
        assert_eq!(decoded.tag, expected_topic_hash);
        assert_eq!(decoded.value, alloy_core::primitives::U256::from(99u64));
    }
}

// Custom/alias types are rejected as indexed fields by design.
// The proc macro cannot distinguish type aliases from custom structs,
// so all Custom types are rejected to guarantee correctness.

// Note: alias types still work as non-indexed fields. Only #[indexed]
// on a custom/alias type is rejected.

type Owner = Address;

#[derive(SolEvent)]
struct OwnershipNonIndexed {
    owner: Owner,
    value: U256,
}

#[test]
fn alias_as_non_indexed_field_is_accepted() {
    let event = OwnershipNonIndexed {
        owner: Address([0xCC; 20]),
        value: U256::from(1u64),
    };
    let mut data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut data);
    assert!(!data.is_empty());
}

// ---------------------------------------------------------------------------
// Indexed array/tuple cross-checks against alloy
// ---------------------------------------------------------------------------

#[derive(SolEvent)]
struct FixedArrayEvent {
    #[indexed]
    values: [u64; 3],
    extra: U256,
}

#[test]
fn indexed_fixed_array_topic_matches_alloy_keccak_abi_encode() {
    use alloy_core::primitives::keccak256;
    use alloy_core::sol_types::SolValue;

    let event = FixedArrayEvent {
        values: [10, 20, 30],
        extra: U256::from(1u64),
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 2);

    // Solidity: keccak256(abi.encode(values))
    // alloy encodes [u64; 3] as three 32-byte words
    let alloy_values: [alloy_core::primitives::U256; 3] = [
        alloy_core::primitives::U256::from(10u64),
        alloy_core::primitives::U256::from(20u64),
        alloy_core::primitives::U256::from(30u64),
    ];
    let encoded = alloy_values.abi_encode();
    let expected = keccak256(&encoded).0;

    assert_eq!(
        topics[1], expected,
        "indexed fixed array topic must match keccak256(abi.encode(values))"
    );
}

#[derive(SolEvent)]
struct TupleEvent {
    #[indexed]
    pair: (u64, u64),
    extra: U256,
}

#[test]
fn indexed_tuple_topic_matches_alloy_keccak_abi_encode() {
    use alloy_core::primitives::keccak256;
    use alloy_core::sol_types::SolValue;

    let event = TupleEvent {
        pair: (100, 200),
        extra: U256::from(1u64),
    };
    let topics = event.topics();
    assert_eq!(topics.len(), 2);

    let alloy_tuple = (
        alloy_core::primitives::U256::from(100u64),
        alloy_core::primitives::U256::from(200u64),
    );
    let encoded = alloy_tuple.abi_encode();
    let expected = keccak256(&encoded).0;

    assert_eq!(
        topics[1], expected,
        "indexed tuple topic must match keccak256(abi.encode(pair))"
    );
}

// ---------------------------------------------------------------------------
// Parameterless events
// ---------------------------------------------------------------------------

#[derive(SolEvent)]
struct Paused;

#[test]
fn parameterless_event_has_only_topic0() {
    let event = Paused;
    let topics = event.topics();
    assert_eq!(
        topics.len(),
        1,
        "parameterless event should have only topic0"
    );
    assert_eq!(topics[0], Paused::TOPIC);

    let mut data = alloc::vec![0u8; event.data_len()];
    event.data_to(&mut data);
    assert!(
        data.is_empty(),
        "parameterless event should have empty data"
    );
}

#[test]
fn parameterless_event_signature() {
    assert_eq!(Paused::SIGNATURE, "Paused()");
}

// ---------------------------------------------------------------------------
// Anonymous events
// ---------------------------------------------------------------------------

#[derive(SolEvent)]
#[anonymous]
struct AnonymousDeposit {
    #[indexed]
    who: Address,
    amount: U256,
}

#[test]
fn anonymous_event_skips_topic0() {
    let event = AnonymousDeposit {
        who: Address([0xAA; 20]),
        amount: U256::from(500u64),
    };
    let topics = event.topics();
    // Anonymous events have no signature topic, just indexed fields
    assert_eq!(
        topics.len(),
        1,
        "anonymous event should have 1 topic (no topic0)"
    );
    // The single topic is the indexed address, right-aligned
    assert_eq!(&topics[0][..12], &[0u8; 12]);
    assert_eq!(&topics[0][12..], &[0xAA; 20]);
}

#[cfg(feature = "abi-gen")]
#[test]
fn anonymous_event_abi_entry_has_anonymous_true() {
    match AnonymousDeposit::abi_item() {
        pvm_contract_types::AbiItem::Event { anonymous, .. } => {
            assert!(anonymous, "anonymous event ABI should have anonymous:true");
        }
        other => panic!("expected AbiItem::Event, got: {other:?}"),
    }
}

#[derive(SolEvent)]
#[anonymous]
struct AnonymousFourIndexed {
    #[indexed]
    a: Address,
    #[indexed]
    b: Address,
    #[indexed]
    c: Address,
    #[indexed]
    d: Address,
}

#[test]
fn anonymous_event_allows_four_indexed_fields() {
    let event = AnonymousFourIndexed {
        a: Address([0x01; 20]),
        b: Address([0x02; 20]),
        c: Address([0x03; 20]),
        d: Address([0x04; 20]),
    };
    let topics = event.topics();
    assert_eq!(
        topics.len(),
        4,
        "anonymous event should allow 4 indexed fields"
    );
}

// ---------------------------------------------------------------------------
// #[alloc] dynamic event emit
// ---------------------------------------------------------------------------

#[derive(SolEvent)]
#[alloc]
struct DynamicLog {
    #[indexed]
    who: Address,
    message: alloc::string::String,
}

#[test]
fn alloc_dynamic_event_emit_e2e() {
    use alloc::string::ToString;
    use pvm_contract_types::{Host, MockHostBuilder, SolDecode};

    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(::std::rc::Rc::new(mock.clone()));

    let event = DynamicLog {
        who: Address([0xCC; 20]),
        message: "hello".to_string(),
    };
    event.emit(&host);

    let events = mock.events();
    assert_eq!(events.len(), 1);

    let (topics, data) = &events[0];
    // topic0 = signature hash
    assert_eq!(topics[0], DynamicLog::TOPIC);
    // topic1 = indexed address, right-aligned
    assert_eq!(&topics[1][..12], &[0u8; 12]);
    assert_eq!(&topics[1][12..], &[0xCC; 20]);
    // data = ABI-encoded "hello"
    let decoded = alloc::string::String::decode(data);
    assert_eq!(decoded, Ok(alloc::string::String::from("hello")));
}
