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
fn error_call() {
    let (_anvil, cast, addr) = deploy("error-handling");
    let c = contract("test-contracts");
    let hex = c.bytecode_hex("error_caller", "release");
    let caller_addr = cast.deploy(&hex, "", &[], DEFAULT_PRIVATE_KEY);
    cast.send(
        &caller_addr,
        "callError(address)",
        &[&addr],
        DEFAULT_PRIVATE_KEY,
    );
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

// --- Receive Handler ---

#[test]
fn receive_handles_plain_ether_transfer() {
    let (_anvil, cast, addr) = deploy("receive");

    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "42");

    let total = cast.call(&addr, "totalReceived()(uint256)", &[]);
    assert_eq!(total, "42");
    let count = cast.call(&addr, "receiveCount()(uint256)", &[]);
    assert_eq!(count, "1");
}

#[test]
fn receive_accumulates_multiple_transfers() {
    let (_anvil, cast, addr) = deploy("receive");

    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "10");
    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "20");
    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "30");

    let total = cast.call(&addr, "totalReceived()(uint256)", &[]);
    assert_eq!(total, "60");
    let count = cast.call(&addr, "receiveCount()(uint256)", &[]);
    assert_eq!(count, "3");
}

#[test]
fn receive_handles_zero_value_empty_calldata() {
    let (_anvil, cast, addr) = deploy("receive");

    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "0");

    let total = cast.call(&addr, "totalReceived()(uint256)", &[]);
    assert_eq!(total, "0");
    let count = cast.call(&addr, "receiveCount()(uint256)", &[]);
    assert_eq!(
        count, "1",
        "receive must fire on empty calldata regardless of value"
    );
}

// --- DSL Receive Handler ---
//
// Mirrors the `receive` tests above but against a DSL-built contract that
// uses `ContractBuilder::receive(...)` instead of the `#[receive]` macro.
// Same wire-level Solidity ABI; same plain-ether-transfer semantics; proves
// the typestate-extended DSL dispatch reaches the registered receive
// handler at runtime.

#[test]
fn receive_dsl_handles_plain_ether_transfer() {
    let (_anvil, cast, addr) = deploy("receive_dsl");

    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "42");

    let total = cast.call(&addr, "totalReceived()(uint256)", &[]);
    assert_eq!(total, "42");
    let count = cast.call(&addr, "receiveCount()(uint256)", &[]);
    assert_eq!(count, "1");
}

#[test]
fn receive_dsl_accumulates_multiple_transfers() {
    let (_anvil, cast, addr) = deploy("receive_dsl");

    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "10");
    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "20");
    cast.send_value_only(&addr, DEFAULT_PRIVATE_KEY, "30");

    let total = cast.call(&addr, "totalReceived()(uint256)", &[]);
    assert_eq!(total, "60");
    let count = cast.call(&addr, "receiveCount()(uint256)", &[]);
    assert_eq!(count, "3");
}

// --- Precompiles ---
//
// The `precompiles` contract forwards to the builtin ecrecover (0x01) and
// P256Verify (0x100) precompiles through the typed SDK wrappers. Unit tests
// mock the call, so these are the only tests that exercise the real
// cryptography and confirm the wrappers build the layout pallet-revive
// expects.

// Published Ethereum ecrecover vector.
const ECR_HASH: &str = "0x456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3";
const ECR_R: &str = "0x9242685bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608";
const ECR_S: &str = "0x4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada";
const ECR_ADDR: &str = "0x7156526fbd7a3c72969b54f64e42c10fbb768c8a";
const ZERO_ADDR: &str = "0x0000000000000000000000000000000000000000";

// go-ethereum's `CallP256Verify` vector — a valid secp256r1 signature.
const P256_HASH: &str = "0x4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4d";
const P256_R: &str = "0xa73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac";
const P256_S: &str = "0x36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d60";
const P256_X: &str = "0x4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff3";
const P256_Y: &str = "0x7618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e";

const RECOVER_SIG: &str = "recover(bytes32,uint8,bytes32,bytes32)(address)";
const VERIFY_P256_SIG: &str = "verifyP256(bytes32,bytes32,bytes32,bytes32,bytes32)(bool)";

#[test]
fn precompile_ecrecover_recovers_signer() {
    let (_anvil, cast, addr) = deploy("precompiles");

    let recovered = cast.call(&addr, RECOVER_SIG, &[ECR_HASH, "28", ECR_R, ECR_S]);
    assert_eq!(recovered.to_lowercase(), ECR_ADDR);
}

#[test]
fn precompile_ecrecover_normalizes_raw_recovery_id() {
    let (_anvil, cast, addr) = deploy("precompiles");

    // v = 1 is the raw recovery id for the same signature; the wrapper lifts
    // it to 28 before handing it to the precompile.
    let recovered = cast.call(&addr, RECOVER_SIG, &[ECR_HASH, "1", ECR_R, ECR_S]);
    assert_eq!(recovered.to_lowercase(), ECR_ADDR);
}

#[test]
fn precompile_ecrecover_failed_recovery_is_zero_address() {
    let (_anvil, cast, addr) = deploy("precompiles");

    // s = 0 is outside the valid range, so the precompile returns empty output.
    let zero = "0x0000000000000000000000000000000000000000000000000000000000000000";
    let recovered = cast.call(&addr, RECOVER_SIG, &[ECR_HASH, "28", ECR_R, zero]);
    assert_eq!(recovered.to_lowercase(), ZERO_ADDR);
}

#[test]
fn precompile_p256_verify_accepts_valid_signature() {
    let (_anvil, cast, addr) = deploy("precompiles");

    let valid = cast.call(
        &addr,
        VERIFY_P256_SIG,
        &[P256_HASH, P256_R, P256_S, P256_X, P256_Y],
    );
    assert_eq!(valid, "true");
}

#[test]
fn precompile_p256_verify_rejects_invalid_signature() {
    let (_anvil, cast, addr) = deploy("precompiles");

    // Wycheproof's "modified r or s, e.g. by adding or subtracting the order
    // of the group" case, which must not verify.
    let hash = "0xbb5a52f42f9c9261ed4361f59422a1e30036e7c32b270c8807a419feca605023";
    let r = "0xd45c5740946b2a147f59262ee6f5bc90bd01ed280528b62b3aed5fc93f06f739";
    let s = "0xb329f479a2bbd0a5c384ee1493b1f5186a87139cac5df4087c134b49156847db";
    let x = "0x2927b10512bae3eddcfe467828128bad2903269919f7086069c8c4df6c732838";
    let y = "0xc7787964eaac00e5921fb1498a60f4606766b3d9685001558d1a974e7341513e";

    let valid = cast.call(&addr, VERIFY_P256_SIG, &[hash, r, s, x, y]);
    assert_eq!(valid, "false");
}

// --- `#[non_reentrant]` modifier: the SDK guard catches a re-entrant call ---

#[test]
fn non_reentrant_guard_blocks_reentry() {
    let (_anvil, cast, addr) = deploy("reentrancy_guard");

    // attemptReentry() makes a real re-entrant call with ALLOW_REENTRY set, so
    // the SDK guard rather than pallet-revive's default reject must catch it.
    let out = cast.send_expect_revert(&addr, "attemptReentry()", &[], DEFAULT_PRIVATE_KEY);
    assert!(
        !out.status.success(),
        "re-entry was not blocked by the guard"
    );

    // Check the revert is the reentrancy error.
    let reentrancy_selector = cast.selector("ReentrancyGuardReentrantCall()");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.to_lowercase().contains(&reentrancy_selector),
        "revert should carry the ReentrancyGuardReentrantCall selector (0x{reentrancy_selector}); got:\n{combined}"
    );
}

// --- `#[non_reentrant]`: the lock is released even when a guarded body exits
// via a raw diverging `return_value`, so a later guarded call in the same
// transaction still succeeds (regression test for the divergence hole). ---

#[test]
fn non_reentrant_guard_unaffected_by_return_value() {
    let (_anvil, cast, addr) = deploy("reentrancy_guard");

    // sequentialGuardedCalls() first invokes a guarded method that exits via a
    // raw return_value (skipping the codegen's post-body unlock), then invokes a
    // guarded method again. The second call must succeed; it would revert with
    // ReentrancyGuardReentrantCall if the divergent exit left the lock set.
    cast.send(&addr, "sequentialGuardedCalls()", &[], DEFAULT_PRIVATE_KEY);

    // Both guarded calls ran their bodies to completion: the diverging one
    // (which commits on its success `return_value`) and the second one after the
    // lock was released.
    assert_eq!(cast.call(&addr, "count()(uint256)", &[]), "2");
}

// --- `#[non_reentrant]`: the guard blocks a genuine cross-contract re-entry
// (A -> attacker -> A.protected()), where the callback carries ALLOW_REENTRY so
// the SDK guard, not pallet-revive's default reject, is what stops it. ---

#[test]
fn non_reentrant_guard_blocks_cross_contract_reentry() {
    let (_anvil, cast, guard_addr) = deploy("reentrancy_guard");
    let c = contract("test-contracts");
    let hex = c.bytecode_hex("reentrancy_attacker", "release");
    let attacker_addr = cast.deploy(&hex, "", &[], DEFAULT_PRIVATE_KEY);

    // protectedCallsOut(attacker) holds the lock, calls attacker.reenter(self),
    // and the attacker calls back into protected() with ALLOW_REENTRY. The
    // re-entrant call must be rejected by the guard, reverting the whole tx.
    let out = cast.send_expect_revert(
        &guard_addr,
        "protectedCallsOut(address)",
        &[&attacker_addr],
        DEFAULT_PRIVATE_KEY,
    );
    assert!(
        !out.status.success(),
        "cross-contract re-entry was not blocked by the guard"
    );

    let reentrancy_selector = cast.selector("ReentrancyGuardReentrantCall()");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.to_lowercase().contains(&reentrancy_selector),
        "revert should carry the ReentrancyGuardReentrantCall selector (0x{reentrancy_selector}); got:\n{combined}"
    );
}
