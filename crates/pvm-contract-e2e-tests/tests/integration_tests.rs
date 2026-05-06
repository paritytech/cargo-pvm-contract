use pvm_contract_e2e_tests::anvil::AnvilPolkadot;
use pvm_contract_e2e_tests::build::contract;
use pvm_contract_e2e_tests::cast::{CastClient, DEFAULT_ADDRESS, DEFAULT_PRIVATE_KEY};

fn deploy(binary_name: &str) -> (AnvilPolkadot, CastClient, String) {
    let c = contract("test-contracts");
    c.build();
    let anvil = AnvilPolkadot::start();
    let cast = CastClient::new(&anvil.rpc_url);
    let hex = c.bytecode_hex(binary_name, "release");
    let address = cast.deploy(&hex, "", &[], DEFAULT_PRIVATE_KEY);
    (anvil, cast, address)
}

#[test]
fn flipper_call_toggle_state() {
    let (_anvil, cast, addr) = deploy("flipper");
    let c = contract("test-contracts");
    let hex = c.bytecode_hex("flipper_call", "release");
    let caller_addr = cast.deploy(&hex, "", &[], DEFAULT_PRIVATE_KEY);
    cast.send(
        &caller_addr,
        "callFlipper(address)",
        &[&addr],
        DEFAULT_PRIVATE_KEY,
    );

    cast.send(&addr, "flip()", &[], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "get()(bool)", &[]);
    assert_eq!(val, "false", "After calling flip state should be false");
}

#[test]
fn point_adder_call() {
    let (_anvil, cast, addr) = deploy("point_adder");
    let c = contract("test-contracts");
    let hex = c.bytecode_hex("point_adder_call", "release");
    let caller_addr = cast.deploy(&hex, "", &[], DEFAULT_PRIVATE_KEY);
    cast.send(
        &caller_addr,
        "callPointAdder(address)",
        &[&addr],
        DEFAULT_PRIVATE_KEY,
    );
}

#[test]
fn flipper_delegate_call_toggle_state() {
    let (_anvil, cast, addr) = deploy("flipper");
    let c = contract("test-contracts");
    let hex = c.bytecode_hex("flipper_delegate", "release");
    let caller_addr = cast.deploy(&hex, "", &[], DEFAULT_PRIVATE_KEY);
    cast.send(
        &caller_addr,
        "delegateFlipper(address)",
        &[&addr],
        DEFAULT_PRIVATE_KEY,
    );

    let val = cast.call(&caller_addr, "get()(bool)", &[]);
    assert_eq!(
        val, "true",
        "After delegate_call flip state should be true in proxy"
    );

    cast.send(&addr, "flip()", &[], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "get()(bool)", &[]);
    assert_eq!(
        val, "true",
        "After calling flip state should be true in original"
    );
}
#[test]
fn flipper_instantiate_call_toggle_state() {
    let (_anvil, cast, addr) = deploy("flipper");
    let c = contract("test-contracts");
    let hex = c.bytecode_hex("flipper_instantiate", "release");
    let caller_addr = cast.deploy(&hex, "", &[], DEFAULT_PRIVATE_KEY);
    cast.send(
        &caller_addr,
        "callFlipper(address)",
        &[&addr],
        DEFAULT_PRIVATE_KEY,
    );

    cast.send(&addr, "flip()", &[], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "get()(bool)", &[]);
    assert_eq!(val, "false", "After calling flip state should be false");
}

#[test]
fn flipper_toggle_state() {
    let (_anvil, cast, addr) = deploy("flipper");

    let val = cast.call(&addr, "get()(bool)", &[]);
    assert_eq!(val, "false", "Initial value should be false");

    cast.send(&addr, "flip()", &[], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "get()(bool)", &[]);
    assert_eq!(val, "true", "After first flip should be true");

    cast.send(&addr, "flip()", &[], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "get()(bool)", &[]);
    assert_eq!(val, "false", "After second flip should be false");
}

#[test]
fn storage_u8_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");

    cast.send(&addr, "setU8(uint8)", &["255"], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getU8()(uint8)", &[]);
    assert_eq!(val, "255");
}

#[test]
fn storage_u16_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");

    cast.send(&addr, "setU16(uint16)", &["65535"], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getU16()(uint16)", &[]);
    assert_eq!(val, "65535");
}

#[test]
fn storage_u32_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");

    cast.send(
        &addr,
        "setU32(uint32)",
        &["4294967295"],
        DEFAULT_PRIVATE_KEY,
    );
    let val = cast.call(&addr, "getU32()(uint32)", &[]);
    assert_eq!(val, "4294967295");
}

#[test]
fn storage_u64_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");

    cast.send(
        &addr,
        "setU64(uint64)",
        &["18446744073709551615"],
        DEFAULT_PRIVATE_KEY,
    );
    let val = cast.call(&addr, "getU64()(uint64)", &[]);
    assert_eq!(val, "18446744073709551615");
}

#[test]
fn storage_u128_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");

    cast.send(
        &addr,
        "setU128(uint128)",
        &["340282366920938463463374607431768211455"],
        DEFAULT_PRIVATE_KEY,
    );
    let val = cast.call(&addr, "getU128()(uint128)", &[]);
    assert_eq!(val, "340282366920938463463374607431768211455");
}

#[test]
fn storage_u256_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");
    let big = "115792089237316195423570985008687907853269984665640564039457584007913129639935";

    cast.send(&addr, "setU256(uint256)", &[big], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getU256()(uint256)", &[]);
    assert_eq!(val, big);
}

#[test]
fn storage_bool_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");

    cast.send(&addr, "setBool(bool)", &["true"], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getBool()(bool)", &[]);
    assert_eq!(val, "true");
}

#[test]
fn storage_address_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");
    let target = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    cast.send(&addr, "setAddress(address)", &[target], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getAddress()(address)", &[]);
    assert_eq!(val.to_lowercase(), target.to_lowercase());
}

#[test]
fn storage_bytes32_roundtrip() {
    let (_anvil, cast, addr) = deploy("storage-types");
    let val_hex = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    cast.send(
        &addr,
        "setBytes32(bytes32)",
        &[val_hex],
        DEFAULT_PRIVATE_KEY,
    );
    let val = cast.call(&addr, "getBytes32()(bytes32)", &[]);
    assert_eq!(val.to_lowercase(), val_hex.to_lowercase());
}

#[test]
fn return_pair_tuple() {
    let (_anvil, cast, addr) = deploy("return-values");

    let val = cast.call(&addr, "getPair()(uint256,bool)", &[]);
    // cast returns tuple as newline-separated values
    let lines: Vec<&str> = val.lines().collect();
    assert_eq!(lines.len(), 2, "Expected 2 return values, got: {val}");
    assert_eq!(lines[0].trim(), "42");
    assert_eq!(lines[1].trim(), "true");
}

#[test]
fn return_triple_tuple() {
    let (_anvil, cast, addr) = deploy("return-values");

    let val = cast.call(&addr, "getTriple()(uint256,address,bool)", &[]);
    let lines: Vec<&str> = val.lines().collect();
    assert_eq!(lines.len(), 3, "Expected 3 return values, got: {val}");
    assert_eq!(lines[0].trim(), "123");
    assert_eq!(
        lines[1].trim().to_lowercase(),
        "0xabababababababababababababababababababab"
    );
    assert_eq!(lines[2].trim(), "false");
}

#[test]
fn return_identity_passthrough() {
    let (_anvil, cast, addr) = deploy("return-values");

    let val = cast.call(&addr, "identity(uint256)(uint256)", &["12345"]);
    assert_eq!(val, "12345");
}

#[test]
fn caller_returns_sender() {
    let (_anvil, cast, addr) = deploy("caller-check");

    let val = cast.call(&addr, "getCaller()(address)", &[]);
    assert_eq!(
        val.to_lowercase(),
        DEFAULT_ADDRESS.to_lowercase(),
        "getCaller should return the transaction sender"
    );
}

#[test]
fn caller_record_and_read() {
    let (_anvil, cast, addr) = deploy("caller-check");

    cast.send(&addr, "recordCaller()", &[], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getLastCaller()(address)", &[]);
    assert_eq!(
        val.to_lowercase(),
        DEFAULT_ADDRESS.to_lowercase(),
        "getLastCaller should return the recorded sender"
    );
}

#[test]
fn error_will_revert() {
    let (_anvil, cast, addr) = deploy("error-handling");

    let output = cast.send_expect_revert(&addr, "willRevert()", &[], DEFAULT_PRIVATE_KEY);
    assert!(!output.status.success(), "willRevert() should revert");
}

#[test]
fn error_will_succeed() {
    let (_anvil, cast, addr) = deploy("error-handling");

    let val = cast.call(&addr, "willSucceed()(bool)", &[]);
    assert_eq!(val, "true");
}

#[test]
fn error_guarded_rejects_zero() {
    let (_anvil, cast, addr) = deploy("error-handling");

    let output = cast.send_expect_revert(&addr, "setGuarded(uint256)", &["0"], DEFAULT_PRIVATE_KEY);
    assert!(!output.status.success(), "setGuarded(0) should revert");
}

#[test]
fn error_guarded_accepts_nonzero() {
    let (_anvil, cast, addr) = deploy("error-handling");

    cast.send(&addr, "setGuarded(uint256)", &["5"], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getGuarded()(uint256)", &[]);
    assert_eq!(val, "5");
}

#[test]
fn events_value_changed() {
    let (_anvil, cast, addr) = deploy("events");

    cast.send(&addr, "setValue(uint256)", &["100"], DEFAULT_PRIVATE_KEY);

    let val = cast.call(&addr, "getValue()(uint256)", &[]);
    assert_eq!(val, "100", "Value should be set to 100");

    let logs = cast.logs(&addr, "ValueChanged(address,uint256,uint256)");
    assert!(!logs.is_empty(), "Should have emitted ValueChanged event");
}

#[test]
fn multi_method_add() {
    let (_anvil, cast, addr) = deploy("multi-method");

    let val = cast.call(&addr, "add(uint256,uint256)(uint256)", &["3", "4"]);
    assert_eq!(val, "7");
}

#[test]
fn multi_method_mul() {
    let (_anvil, cast, addr) = deploy("multi-method");

    let val = cast.call(&addr, "mul(uint256,uint256)(uint256)", &["3", "4"]);
    assert_eq!(val, "12");
}

#[test]
fn multi_method_is_zero() {
    let (_anvil, cast, addr) = deploy("multi-method");

    let val = cast.call(&addr, "isZero(uint256)(bool)", &["0"]);
    assert_eq!(val, "true");

    let val = cast.call(&addr, "isZero(uint256)(bool)", &["1"]);
    assert_eq!(val, "false");
}

#[test]
fn multi_method_counter() {
    let (_anvil, cast, addr) = deploy("multi-method");

    let val = cast.call(&addr, "getCounter()(uint256)", &[]);
    assert_eq!(val, "0", "Counter should start at 0");

    cast.send(&addr, "increment()", &[], DEFAULT_PRIVATE_KEY);
    cast.send(&addr, "increment()", &[], DEFAULT_PRIVATE_KEY);
    cast.send(&addr, "increment()", &[], DEFAULT_PRIVATE_KEY);

    let val = cast.call(&addr, "getCounter()(uint256)", &[]);
    assert_eq!(val, "3", "Counter should be 3 after 3 increments");

    cast.send(&addr, "reset()", &[], DEFAULT_PRIVATE_KEY);
    let val = cast.call(&addr, "getCounter()(uint256)", &[]);
    assert_eq!(val, "0", "Counter should be 0 after reset");
}

// --- Dynamic Types ---

#[test]
fn dynamic_string_length() {
    let (_anvil, cast, addr) = deploy("dynamic-types");

    let val = cast.call(&addr, "getStringLength(string)(uint256)", &["hello world"]);
    assert_eq!(val, "11");
}

#[test]
fn dynamic_echo_string() {
    let (_anvil, cast, addr) = deploy("dynamic-types");

    let val = cast.call(&addr, "echoString()(string)", &[]);
    // cast wraps string returns in quotes
    let val = val.trim_matches('"');
    assert_eq!(val, "hello world");
}

#[test]
fn dynamic_bytes_length() {
    let (_anvil, cast, addr) = deploy("dynamic-types");

    let val = cast.call(&addr, "getBytesLength(bytes)(uint256)", &["0xDEADBEEF"]);
    assert_eq!(val, "4");
}

#[test]
fn dynamic_echo_bytes() {
    let (_anvil, cast, addr) = deploy("dynamic-types");

    let val = cast.call(&addr, "echoBytes()(bytes)", &[]);
    assert_eq!(val.to_lowercase(), "0xdeadbeef");
}

#[test]
fn dynamic_sum_array() {
    let (_anvil, cast, addr) = deploy("dynamic-types");

    let val = cast.call(&addr, "sumArray(uint256[])(uint256)", &["[1,2,3]"]);
    assert_eq!(val, "6");
}

#[test]
fn dynamic_get_array() {
    let (_anvil, cast, addr) = deploy("dynamic-types");

    let val = cast.call(&addr, "getArray()(uint256[])", &[]);
    // cast returns arrays as newline-separated or bracket-formatted values
    assert!(
        val.contains("10") && val.contains("20") && val.contains("30"),
        "Expected array [10, 20, 30], got: {val}"
    );
}

// --- Composite Types ---

#[test]
fn composite_sum_fixed_array() {
    let (_anvil, cast, addr) = deploy("composite-types");

    let val = cast.call(&addr, "sumFixedArray(uint256[3])(uint256)", &["[10,20,30]"]);
    assert_eq!(val, "60");
}

#[test]
fn composite_get_fixed_array() {
    let (_anvil, cast, addr) = deploy("composite-types");

    let val = cast.call(&addr, "getFixedArray()(uint256[3])", &[]);
    assert!(
        val.contains("10") && val.contains("20") && val.contains("30"),
        "Expected array [10, 20, 30], got: {val}"
    );
}

#[test]
fn composite_tuple_true() {
    let (_anvil, cast, addr) = deploy("composite-types");

    let val = cast.call(
        &addr,
        "processTuple((uint256,bool))(uint256)",
        &["(42,true)"],
    );
    assert_eq!(val, "42");
}

#[test]
fn composite_tuple_false() {
    let (_anvil, cast, addr) = deploy("composite-types");

    let val = cast.call(
        &addr,
        "processTuple((uint256,bool))(uint256)",
        &["(42,false)"],
    );
    assert_eq!(val, "0");
}

// --- Constructor Arguments ---

fn deploy_constructor_args(owner: &str, supply: &str) -> (AnvilPolkadot, CastClient, String) {
    let c = contract("test-contracts");
    c.build();
    let anvil = AnvilPolkadot::start();
    let cast = CastClient::new(&anvil.rpc_url);
    let hex = c.bytecode_hex("constructor-args", "release");
    let address = cast.deploy(
        &hex,
        "constructor(address,uint256)",
        &[owner, supply],
        DEFAULT_PRIVATE_KEY,
    );
    (anvil, cast, address)
}

#[test]
fn constructor_args_sets_owner_and_supply() {
    let owner = DEFAULT_ADDRESS;
    let supply = "1000000";
    let (_anvil, cast, addr) = deploy_constructor_args(owner, supply);

    let got_owner = cast.call(&addr, "getOwner()(address)", &[]);
    assert_eq!(
        got_owner.to_lowercase(),
        owner.to_lowercase(),
        "Constructor should set owner"
    );

    let got_supply = cast.call(&addr, "getInitialSupply()(uint256)", &[]);
    assert_eq!(got_supply, supply, "Constructor should set initial supply");
}

#[test]
fn constructor_args_different_values() {
    let owner = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let supply = "999";
    let (_anvil, cast, addr) = deploy_constructor_args(owner, supply);

    let got_owner = cast.call(&addr, "getOwner()(address)", &[]);
    assert_eq!(
        got_owner.to_lowercase(),
        owner.to_lowercase(),
        "Constructor should set different owner"
    );

    let got_supply = cast.call(&addr, "getInitialSupply()(uint256)", &[]);
    assert_eq!(
        got_supply, supply,
        "Constructor should set different supply"
    );
}

#[test]
fn constructor_args_zero_supply() {
    let (_anvil, cast, addr) = deploy_constructor_args(DEFAULT_ADDRESS, "0");

    let got_supply = cast.call(&addr, "getInitialSupply()(uint256)", &[]);
    assert_eq!(got_supply, "0", "Constructor should handle zero supply");
}

// --- Payable Enforcement ---

#[test]
fn payable_deposit_accepts_value() {
    let (_anvil, cast, addr) = deploy("payable");
    cast.send_with_value(&addr, "deposit()", &[], DEFAULT_PRIVATE_KEY, "100");
    let bal = cast.call(&addr, "balanceOf(address)(uint256)", &[DEFAULT_ADDRESS]);
    assert_eq!(bal, "100");
}

#[test]
fn payable_deposit_to_accepts_value() {
    let (_anvil, cast, addr) = deploy("payable");
    let recipient = "0x0000000000000000000000000000000000000001";
    cast.send_with_value(
        &addr,
        "depositTo(address)",
        &[recipient],
        DEFAULT_PRIVATE_KEY,
        "50",
    );
    let bal = cast.call(&addr, "balanceOf(address)(uint256)", &[recipient]);
    assert_eq!(bal, "50");
}

#[test]
fn payable_deposit_with_zero_value_ok() {
    let (_anvil, cast, addr) = deploy("payable");
    cast.send_with_value(&addr, "deposit()", &[], DEFAULT_PRIVATE_KEY, "0");
    let bal = cast.call(&addr, "balanceOf(address)(uint256)", &[DEFAULT_ADDRESS]);
    assert_eq!(bal, "0");
}

#[test]
fn non_payable_transfer_rejects_value() {
    let (_anvil, cast, addr) = deploy("payable");
    let output = cast.send_with_value_expect_revert(
        &addr,
        "transfer(address,uint256)",
        &["0x0000000000000000000000000000000000000001", "0"],
        DEFAULT_PRIVATE_KEY,
        "1",
    );
    assert!(
        !output.status.success(),
        "non-payable transfer should revert when value is sent",
    );
}

#[test]
fn non_payable_transfer_accepts_zero_value() {
    let (_anvil, cast, addr) = deploy("payable");
    cast.send_with_value(&addr, "deposit()", &[], DEFAULT_PRIVATE_KEY, "100");
    let recipient = "0x0000000000000000000000000000000000000001";
    cast.send(
        &addr,
        "transfer(address,uint256)",
        &[recipient, "50"],
        DEFAULT_PRIVATE_KEY,
    );
    let bal = cast.call(&addr, "balanceOf(address)(uint256)", &[recipient]);
    assert_eq!(bal, "50");
}

#[test]
fn non_payable_constructor_rejects_value() {
    let c = contract("test-contracts");
    c.build();
    let anvil = AnvilPolkadot::start();
    let cast = CastClient::new(&anvil.rpc_url);
    let hex = c.bytecode_hex("payable", "release");
    let result = cast.deploy_with_value(&hex, "", &[], DEFAULT_PRIVATE_KEY, "1");
    assert!(
        result.is_err(),
        "non-payable constructor should reject value"
    );
    drop(anvil);
}

#[test]
fn non_payable_constructor_accepts_zero_value() {
    let (_anvil, _cast, addr) = deploy("payable");
    assert!(!addr.is_empty(), "deploy without value should succeed");
}
