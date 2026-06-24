#![cfg(not(feature = "abi-gen"))]

extern crate alloc;

use pvm_contract_sdk::SolDecode;
use pvm_contract_sdk::SolEncode;

pvm_contract_sdk::abi_import! {
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;

    enum Light {
        Red,
        Yellow,
        Green
    }

    interface Traffic {
        function setLight(Light light) external;
        function currentLight() external view returns (Light);
    }
}

#[test]
fn imported_enum_encodes_as_uint8() {
    assert_eq!(<Light as SolEncode>::SOL_NAME, "uint8");

    let mut buf = [0u8; 32];
    Light::Green.encode_to(&mut buf);
    let mut expected = [0u8; 32];
    expected[31] = 2;
    assert_eq!(buf, expected);
}

#[test]
fn imported_enum_roundtrips() {
    for (variant, discriminant) in [(Light::Red, 0u8), (Light::Yellow, 1), (Light::Green, 2)] {
        let mut buf = [0u8; 32];
        variant.encode_to(&mut buf);
        assert_eq!(Light::decode(&buf).unwrap(), variant);
        assert_eq!(buf[31], discriminant);
    }
}

#[test]
fn imported_enum_decode_rejects_out_of_range() {
    let mut buf = [0u8; 32];
    buf[31] = 3;
    assert!(Light::decode(&buf).is_err());
}

#[test]
fn imported_interface_constructs_with_enum_methods() {
    let target = pvm_contract_sdk::Address::from([0x11; 20]);
    let _ = traffic::Traffic::from_address(target).current_light();
    let _ = traffic::Traffic::from_address(target).set_light(Light::Yellow);
}

pvm_contract_sdk::abi_import! {
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;

    interface Signal {
        enum Phase {
            Idle,
            Active,
            Done
        }
        function setPhase(Phase phase) external;
        function phase() external view returns (Phase);
    }
}

#[test]
fn body_declared_enum_roundtrips_and_rejects_out_of_range() {
    use signal::Phase;

    let mut buf = [0u8; 32];
    Phase::Active.encode_to(&mut buf);
    assert_eq!(buf[31], 1);
    assert_eq!(Phase::decode(&buf).unwrap(), Phase::Active);

    buf[31] = 9;
    assert!(Phase::decode(&buf).is_err());

    let target = pvm_contract_sdk::Address::from([0x22; 20]);
    let _ = signal::Signal::from_address(target).set_phase(Phase::Done);
    let _ = signal::Signal::from_address(target).phase();
}
