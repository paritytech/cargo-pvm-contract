use pvm_contract_sdk::{Address, SolError};
use pvm_contract_sdk::{U256, const_selector};

#[derive(SolError)]
struct InsufficientBalance {
    account: Address,
    required: U256,
    available: U256,
}

#[test]
fn selector_matches_keccak() {
    // keccak256("InsufficientBalance(address,uint256,uint256)")[0:4]
    let expected = pvm_contract_sdk::const_selector("InsufficientBalance(address,uint256,uint256)");
    assert_eq!(InsufficientBalance::SELECTOR, expected);
}

#[test]
fn signature_is_canonical() {
    assert_eq!(
        InsufficientBalance::SIGNATURE,
        "InsufficientBalance(address,uint256,uint256)"
    );
}

#[test]
fn encode_params_size() {
    let error = InsufficientBalance {
        account: Address([0xAB; 20]),
        required: U256::from(1000u64),
        available: U256::from(500u64),
    };
    let mut buf = [0u8; 256];
    let len = error.encode_to(&mut buf);
    assert_eq!(len, 100); // selector + 3 x 32 bytes
}

mod alloy_cross_check {
    use pvm_contract_sdk::SolError;

    alloy_core::sol! { error InsufficientBalance(address account, uint256 required, uint256 available); }

    #[test]
    fn encoding_matches_alloy() {
        use alloy_core::sol_types::SolError as AlloySolError;
        use pvm_contract_sdk::Address;
        use pvm_contract_sdk::U256;

        // Encode with our SolError derive
        let error = crate::InsufficientBalance {
            account: Address([0xAB; 20]),
            required: U256::from(1000u64),
            available: U256::from(500u64),
        };
        let mut buf = [0u8; 256];
        let len = error.encode_to(&mut buf);

        // Encode with alloy's sol! error
        let alloy_encoded = InsufficientBalance {
            account: alloy_core::primitives::Address::from([0xAB; 20]),
            required: alloy_core::primitives::U256::from(1000u64),
            available: alloy_core::primitives::U256::from(500u64),
        }
        .abi_encode();

        assert_eq!(&buf[..len], &alloy_encoded[..]);
    }
}

#[test]
fn encoded_size_includes_selector() {
    let error = InsufficientBalance {
        account: Address([0xAB; 20]),
        required: U256::from(1000u64),
        available: U256::from(500u64),
    };
    assert_eq!(error.encoded_size(), 4 + 96);
}

#[test]
fn revert_data_includes_selector_and_params() {
    let error = InsufficientBalance {
        account: Address([0xAB; 20]),
        required: U256::from(1000u64),
        available: U256::from(500u64),
    };
    let mut buf = [0u8; 256];
    let len = error.encode_to(&mut buf);
    assert_eq!(len, 100); // 4 selector + 96 params
    assert_eq!(&buf[0..4], &InsufficientBalance::SELECTOR);
}

// Zero-field error
#[derive(SolError)]
struct Unauthorized;

#[test]
fn zero_field_error_signature() {
    assert_eq!(Unauthorized::SIGNATURE, "Unauthorized()");
}

#[test]
fn zero_field_error_revert_data() {
    let error = Unauthorized;
    let mut buf = [0u8; 256];
    let len = error.encode_to(&mut buf);
    assert_eq!(len, 4);
    assert_eq!(&buf[0..4], &Unauthorized::SELECTOR);
}

// Type alias resolution
type Amount = U256;

#[derive(SolError)]
struct OverLimit {
    limit: Amount,
}

#[test]
fn type_alias_resolves_in_signature() {
    // Amount = U256, so SOL_NAME = "uint256"
    assert_eq!(OverLimit::SIGNATURE, "OverLimit(uint256)");
}

#[test]
fn type_alias_resolves_in_selector() {
    let expected = pvm_contract_sdk::const_selector("OverLimit(uint256)");
    assert_eq!(OverLimit::SELECTOR, expected);
}

// --- Nested custom type ---

#[derive(pvm_contract_macros::SolType, Debug, PartialEq, Eq, Clone, Copy)]
struct Point {
    x: u64,
    y: u64,
}

#[derive(SolError)]
struct PointError {
    origin: Point,
    value: U256,
}

#[test]
fn nested_custom_type_signature() {
    // Point encodes as (uint64,uint64)
    assert_eq!(PointError::SIGNATURE, "PointError((uint64,uint64),uint256)");
}

#[test]
fn nested_custom_type_selector() {
    let expected = pvm_contract_sdk::const_selector("PointError((uint64,uint64),uint256)");
    assert_eq!(PointError::SELECTOR, expected);
}

#[test]
fn nested_custom_type_encode_params() {
    let error = PointError {
        origin: Point { x: 1, y: 2 },
        value: U256::from(42u64),
    };
    let mut buf = [0u8; 256];
    let len = error.encode_to(&mut buf);
    // Point = 64 bytes (2 x u64 @ 32 each) + U256 = 32 bytes = 96 total
    assert_eq!(len - 4, 96);

    // Verify Point fields are encoded correctly
    // x = 1, big-endian in 32 bytes
    assert_eq!(buf[31 + 4], 1);
    // y = 2
    assert_eq!(buf[63 + 4], 2);
    // value = 42
    assert_eq!(buf[95 + 4], 42);
}

#[test]
fn nested_custom_type_roundtrip_with_revert_data() {
    let error = PointError {
        origin: Point { x: 10, y: 20 },
        value: U256::from(100u64),
    };
    let mut buf = [0u8; 256];
    let len = error.encode_to(&mut buf);
    assert_eq!(len, 100); // 4 selector + 96 params
    assert_eq!(&buf[0..4], &PointError::SELECTOR);
}

#[test]
fn sol_error_enum_dispatches_correctly() {
    #[derive(SolError)]
    struct ErrA;

    #[derive(SolError)]
    struct ErrB;

    #[derive(SolError)]
    enum TestError {
        A(ErrA),
        B(ErrB),
        Panic(Panic),
        Revert(RevertString),
    }

    let mut buf = [0u8; 256];

    // Custom errors
    let err: TestError = ErrA.into();
    let len = err.encode_to(&mut buf);
    assert_eq!(len, 4);
    assert_eq!(&buf[0..4], &const_selector("ErrA()"));

    let err: TestError = ErrB.into();
    let len = err.encode_to(&mut buf);
    assert_eq!(len, 4);
    assert_eq!(&buf[0..4], &const_selector("ErrB()"));

    let err: TestError = Panic::Overflow.into();
    let len = err.encode_to(&mut buf);
    assert_eq!(len, 36);
    assert_eq!(&buf[0..4], &Panic::SELECTOR);
    assert_eq!(buf[35], 0x11);

    // Auto-injected RevertString
    let err: TestError = RevertString("fail".to_string()).into();
    let len = err.encode_to(&mut buf);
    assert_eq!(&buf[0..4], &RevertString::SELECTOR);
    assert!(len > 4);
}

use pvm_contract_types::{Panic, RevertString, SolDefaultError};

#[test]
fn sol_error_enum_question_mark_propagation() {
    #[derive(SolError)]
    struct CustomErr;

    #[derive(SolError)]
    enum MyError {
        Custom(CustomErr),
        Panic(Panic),
        Revert(RevertString),
    }

    // Verify From impls work for ? propagation
    fn returns_custom() -> Result<(), MyError> {
        Err(CustomErr)?
    }
    fn returns_panic() -> Result<(), MyError> {
        Err(Panic::Overflow)?
    }
    fn returns_revert() -> Result<(), MyError> {
        Err(RevertString("nope".to_string()))?
    }

    assert!(returns_custom().is_err());
    assert!(returns_panic().is_err());
    assert!(returns_revert().is_err());
}

#[test]
fn sol_default_error_question_mark_propagation() {
    fn checked_sub(a: u64, b: u64) -> Result<u64, SolDefaultError> {
        a.checked_sub(b).ok_or(Panic::Overflow.into())
    }

    fn do_transfer(balance: u64, amount: u64) -> Result<u64, SolDefaultError> {
        let new_balance = checked_sub(balance, amount)?;
        Ok(new_balance)
    }

    match do_transfer(100, 50) {
        Ok(val) => assert_eq!(val, 50),
        Err(_) => panic!("expected success"),
    }
    assert!(do_transfer(10, 20).is_err());

    // Verify the error encodes correctly
    match do_transfer(10, 20) {
        Err(err) => {
            let mut buf = [0u8; 256];
            let len = err.encode_to(&mut buf);
            assert_eq!(&buf[0..4], &Panic::SELECTOR);
            assert_eq!(buf[35], 0x11); // Overflow code
            assert_eq!(len, 36);
        }
        Ok(_) => panic!("expected error"),
    }
}

#[test]
fn decode_at_roundtrips_struct_with_fields() {
    #[derive(SolError, Debug, PartialEq, Eq)]
    struct RoundtripError {
        account: Address,
        required: U256,
    }
    let original = RoundtripError {
        account: Address([0x42; 20]),
        required: U256::from(1000u64),
    };
    let mut buf = [0u8; 512];
    let len = original.encode_to(&mut buf);
    assert_eq!(len, 68);

    let decoded = RoundtripError::decode_at(&buf[..len], 0)
        .expect("decode_at returned DecodeError")
        .expect("selector did not match");

    assert_eq!(decoded, original);
}
