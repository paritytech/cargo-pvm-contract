//! Host backend abstraction for PVM smart contracts.
//!
//! Provides [`HostApi`], a trait that mirrors `pallet_revive_uapi::HostFn` 1:1
//! but is owned by this crate (not sealed), enabling custom implementations:
//!
//! - [`PolkaVmHost`]: delegates to `HostFnImpl` on riscv64, `unimplemented!()` stubs elsewhere
//! - [`MockHost`](super::MockHost): test backend with configurable state (requires `std` feature)

pub use pallet_revive_uapi::{CallFlags, ReturnErrorCode, ReturnFlags, StorageFlags};

/// Result type for host operations that can fail.
pub type HostResult = core::result::Result<(), ReturnErrorCode>;

/// Host API trait mirroring all methods from `pallet_revive_uapi::HostFn`.
///
/// Unlike `HostFn`, this trait is not sealed — any type can implement it.
/// All methods are static (no `&self`) to match the upstream API.
#[allow(clippy::too_many_arguments)]
pub trait HostApi {
    fn address(output: &mut [u8; 20]);
    fn get_immutable_data(output: &mut &mut [u8]);
    fn set_immutable_data(data: &[u8]);
    fn balance(output: &mut [u8; 32]);
    fn balance_of(addr: &[u8; 20], output: &mut [u8; 32]);
    fn chain_id(output: &mut [u8; 32]);
    fn gas_price() -> u64;
    fn base_fee(output: &mut [u8; 32]);
    fn call_data_size() -> u64;
    fn call(
        flags: CallFlags,
        callee: &[u8; 20],
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit: &[u8; 32],
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult;
    fn call_evm(
        flags: CallFlags,
        callee: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult;
    fn caller(output: &mut [u8; 20]);
    fn origin(output: &mut [u8; 20]);
    fn code_hash(addr: &[u8; 20], output: &mut [u8; 32]);
    fn code_size(addr: &[u8; 20]) -> u64;
    fn delegate_call(
        flags: CallFlags,
        address: &[u8; 20],
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit_limit: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult;
    fn delegate_call_evm(
        flags: CallFlags,
        address: &[u8; 20],
        gas: u64,
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult;
    fn deposit_event(topics: &[[u8; 32]], data: &[u8]);
    fn get_storage(flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult;
    fn hash_keccak_256(input: &[u8], output: &mut [u8; 32]);
    fn call_data_copy(output: &mut [u8], offset: u32);
    fn call_data_load(output: &mut [u8; 32], offset: u32);
    fn instantiate(
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit: &[u8; 32],
        value: &[u8; 32],
        input: &[u8],
        address: Option<&mut [u8; 20]>,
        output: Option<&mut &mut [u8]>,
        salt: Option<&[u8; 32]>,
    ) -> HostResult;
    fn now(output: &mut [u8; 32]);
    fn gas_limit() -> u64;
    fn return_value(flags: ReturnFlags, return_value: &[u8]) -> !;
    fn set_storage(flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32>;
    fn set_storage_or_clear(flags: StorageFlags, key: &[u8; 32], value: &[u8; 32]) -> Option<u32>;
    fn get_storage_or_zero(flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]);
    fn value_transferred(output: &mut [u8; 32]);
    fn return_data_size() -> u64;
    fn return_data_copy(output: &mut &mut [u8], offset: u32);
    fn gas_left() -> u64;
    fn block_author(output: &mut [u8; 20]);
    fn block_number(output: &mut [u8; 32]);
    fn block_hash(block_number: &[u8; 32], output: &mut [u8; 32]);
    fn consume_all_gas() -> !;
    fn terminate(beneficiary: &[u8; 20]) -> !;
}

/// Real host backend for PolkaVM contracts.
///
/// On `riscv64`, all methods delegate to `pallet_revive_uapi::HostFnImpl`.
/// On other targets, methods are `unimplemented!()` stubs that compile but
/// panic at runtime — this allows contract code to compile on the host
/// for ABI generation and type checking.
pub struct PolkaVmHost;

#[cfg(target_arch = "riscv64")]
use pallet_revive_uapi::HostFn as _;

#[cfg(target_arch = "riscv64")]
impl HostApi for PolkaVmHost {
    fn address(output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::address(output)
    }
    fn get_immutable_data(output: &mut &mut [u8]) {
        pallet_revive_uapi::HostFnImpl::get_immutable_data(output)
    }
    fn set_immutable_data(data: &[u8]) {
        pallet_revive_uapi::HostFnImpl::set_immutable_data(data)
    }
    fn balance(output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::balance(output)
    }
    fn balance_of(addr: &[u8; 20], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::balance_of(addr, output)
    }
    fn chain_id(output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::chain_id(output)
    }
    fn gas_price() -> u64 {
        pallet_revive_uapi::HostFnImpl::gas_price()
    }
    fn base_fee(output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::base_fee(output)
    }
    fn call_data_size() -> u64 {
        pallet_revive_uapi::HostFnImpl::call_data_size()
    }
    fn call(
        flags: CallFlags,
        callee: &[u8; 20],
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit: &[u8; 32],
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        pallet_revive_uapi::HostFnImpl::call(
            flags,
            callee,
            ref_time_limit,
            proof_size_limit,
            deposit,
            value,
            input_data,
            output,
        )
    }
    fn call_evm(
        flags: CallFlags,
        callee: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        pallet_revive_uapi::HostFnImpl::call_evm(flags, callee, gas, value, input_data, output)
    }
    fn caller(output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::caller(output)
    }
    fn origin(output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::origin(output)
    }
    fn code_hash(addr: &[u8; 20], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::code_hash(addr, output)
    }
    fn code_size(addr: &[u8; 20]) -> u64 {
        pallet_revive_uapi::HostFnImpl::code_size(addr)
    }
    fn delegate_call(
        flags: CallFlags,
        address: &[u8; 20],
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit_limit: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        pallet_revive_uapi::HostFnImpl::delegate_call(
            flags,
            address,
            ref_time_limit,
            proof_size_limit,
            deposit_limit,
            input_data,
            output,
        )
    }
    fn delegate_call_evm(
        flags: CallFlags,
        address: &[u8; 20],
        gas: u64,
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        pallet_revive_uapi::HostFnImpl::delegate_call_evm(flags, address, gas, input_data, output)
    }
    fn deposit_event(topics: &[[u8; 32]], data: &[u8]) {
        pallet_revive_uapi::HostFnImpl::deposit_event(topics, data)
    }
    fn get_storage(flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult {
        pallet_revive_uapi::HostFnImpl::get_storage(flags, key, output)
    }
    fn hash_keccak_256(input: &[u8], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::hash_keccak_256(input, output)
    }
    fn call_data_copy(output: &mut [u8], offset: u32) {
        pallet_revive_uapi::HostFnImpl::call_data_copy(output, offset)
    }
    fn call_data_load(output: &mut [u8; 32], offset: u32) {
        pallet_revive_uapi::HostFnImpl::call_data_load(output, offset)
    }
    fn instantiate(
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit: &[u8; 32],
        value: &[u8; 32],
        input: &[u8],
        address: Option<&mut [u8; 20]>,
        output: Option<&mut &mut [u8]>,
        salt: Option<&[u8; 32]>,
    ) -> HostResult {
        pallet_revive_uapi::HostFnImpl::instantiate(
            ref_time_limit,
            proof_size_limit,
            deposit,
            value,
            input,
            address,
            output,
            salt,
        )
    }
    fn now(output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::now(output)
    }
    fn gas_limit() -> u64 {
        pallet_revive_uapi::HostFnImpl::gas_limit()
    }
    fn return_value(flags: ReturnFlags, return_value: &[u8]) -> ! {
        pallet_revive_uapi::HostFnImpl::return_value(flags, return_value)
    }
    fn set_storage(flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32> {
        pallet_revive_uapi::HostFnImpl::set_storage(flags, key, value)
    }
    fn set_storage_or_clear(flags: StorageFlags, key: &[u8; 32], value: &[u8; 32]) -> Option<u32> {
        pallet_revive_uapi::HostFnImpl::set_storage_or_clear(flags, key, value)
    }
    fn get_storage_or_zero(flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::get_storage_or_zero(flags, key, output)
    }
    fn value_transferred(output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::value_transferred(output)
    }
    fn return_data_size() -> u64 {
        pallet_revive_uapi::HostFnImpl::return_data_size()
    }
    fn return_data_copy(output: &mut &mut [u8], offset: u32) {
        pallet_revive_uapi::HostFnImpl::return_data_copy(output, offset)
    }
    fn gas_left() -> u64 {
        pallet_revive_uapi::HostFnImpl::gas_left()
    }
    fn block_author(output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::block_author(output)
    }
    fn block_number(output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::block_number(output)
    }
    fn block_hash(block_number: &[u8; 32], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::block_hash(block_number, output)
    }
    fn consume_all_gas() -> ! {
        pallet_revive_uapi::HostFnImpl::consume_all_gas()
    }
    fn terminate(beneficiary: &[u8; 20]) -> ! {
        pallet_revive_uapi::HostFnImpl::terminate(beneficiary)
    }
}

#[cfg(not(target_arch = "riscv64"))]
impl HostApi for PolkaVmHost {
    fn address(_output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::address is only available on PolkaVM")
    }
    fn get_immutable_data(_output: &mut &mut [u8]) {
        unimplemented!("PolkaVmHost::get_immutable_data is only available on PolkaVM")
    }
    fn set_immutable_data(_data: &[u8]) {
        unimplemented!("PolkaVmHost::set_immutable_data is only available on PolkaVM")
    }
    fn balance(_output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::balance is only available on PolkaVM")
    }
    fn balance_of(_addr: &[u8; 20], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::balance_of is only available on PolkaVM")
    }
    fn chain_id(_output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::chain_id is only available on PolkaVM")
    }
    fn gas_price() -> u64 {
        unimplemented!("PolkaVmHost::gas_price is only available on PolkaVM")
    }
    fn base_fee(_output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::base_fee is only available on PolkaVM")
    }
    fn call_data_size() -> u64 {
        unimplemented!("PolkaVmHost::call_data_size is only available on PolkaVM")
    }
    fn call(
        _flags: CallFlags,
        _callee: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::call is only available on PolkaVM")
    }
    fn call_evm(
        _flags: CallFlags,
        _callee: &[u8; 20],
        _gas: u64,
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::call_evm is only available on PolkaVM")
    }
    fn caller(_output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::caller is only available on PolkaVM")
    }
    fn origin(_output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::origin is only available on PolkaVM")
    }
    fn code_hash(_addr: &[u8; 20], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::code_hash is only available on PolkaVM")
    }
    fn code_size(_addr: &[u8; 20]) -> u64 {
        unimplemented!("PolkaVmHost::code_size is only available on PolkaVM")
    }
    fn delegate_call(
        _flags: CallFlags,
        _address: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit_limit: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::delegate_call is only available on PolkaVM")
    }
    fn delegate_call_evm(
        _flags: CallFlags,
        _address: &[u8; 20],
        _gas: u64,
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::delegate_call_evm is only available on PolkaVM")
    }
    fn deposit_event(_topics: &[[u8; 32]], _data: &[u8]) {
        unimplemented!("PolkaVmHost::deposit_event is only available on PolkaVM")
    }
    fn get_storage(_flags: StorageFlags, _key: &[u8], _output: &mut &mut [u8]) -> HostResult {
        unimplemented!("PolkaVmHost::get_storage is only available on PolkaVM")
    }
    fn hash_keccak_256(_input: &[u8], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::hash_keccak_256 is only available on PolkaVM")
    }
    fn call_data_copy(_output: &mut [u8], _offset: u32) {
        unimplemented!("PolkaVmHost::call_data_copy is only available on PolkaVM")
    }
    fn call_data_load(_output: &mut [u8; 32], _offset: u32) {
        unimplemented!("PolkaVmHost::call_data_load is only available on PolkaVM")
    }
    fn instantiate(
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input: &[u8],
        _address: Option<&mut [u8; 20]>,
        _output: Option<&mut &mut [u8]>,
        _salt: Option<&[u8; 32]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::instantiate is only available on PolkaVM")
    }
    fn now(_output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::now is only available on PolkaVM")
    }
    fn gas_limit() -> u64 {
        unimplemented!("PolkaVmHost::gas_limit is only available on PolkaVM")
    }
    fn return_value(_flags: ReturnFlags, _return_value: &[u8]) -> ! {
        unimplemented!("PolkaVmHost::return_value is only available on PolkaVM")
    }
    fn set_storage(_flags: StorageFlags, _key: &[u8], _value: &[u8]) -> Option<u32> {
        unimplemented!("PolkaVmHost::set_storage is only available on PolkaVM")
    }
    fn set_storage_or_clear(
        _flags: StorageFlags,
        _key: &[u8; 32],
        _value: &[u8; 32],
    ) -> Option<u32> {
        unimplemented!("PolkaVmHost::set_storage_or_clear is only available on PolkaVM")
    }
    fn get_storage_or_zero(_flags: StorageFlags, _key: &[u8; 32], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::get_storage_or_zero is only available on PolkaVM")
    }
    fn value_transferred(_output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::value_transferred is only available on PolkaVM")
    }
    fn return_data_size() -> u64 {
        unimplemented!("PolkaVmHost::return_data_size is only available on PolkaVM")
    }
    fn return_data_copy(_output: &mut &mut [u8], _offset: u32) {
        unimplemented!("PolkaVmHost::return_data_copy is only available on PolkaVM")
    }
    fn gas_left() -> u64 {
        unimplemented!("PolkaVmHost::gas_left is only available on PolkaVM")
    }
    fn block_author(_output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::block_author is only available on PolkaVM")
    }
    fn block_number(_output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::block_number is only available on PolkaVM")
    }
    fn block_hash(_block_number: &[u8; 32], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::block_hash is only available on PolkaVM")
    }
    fn consume_all_gas() -> ! {
        unimplemented!("PolkaVmHost::consume_all_gas is only available on PolkaVM")
    }
    fn terminate(_beneficiary: &[u8; 20]) -> ! {
        unimplemented!("PolkaVmHost::terminate is only available on PolkaVM")
    }
}
