extern crate alloc;

use alloc::vec::Vec;

use crate::CallFlags;
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
use crate::{HostFn, api};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    Reverted,
    Trapped,
    TransferFailed,
    OutOfResources,
    Unknown,
}

pub type CallResult<T> = core::result::Result<T, CallError>;

impl From<pallet_revive_uapi::ReturnErrorCode> for CallError {
    fn from(code: pallet_revive_uapi::ReturnErrorCode) -> Self {
        match code {
            pallet_revive_uapi::ReturnErrorCode::CalleeReverted => CallError::Reverted,
            pallet_revive_uapi::ReturnErrorCode::CalleeTrapped => CallError::Trapped,
            pallet_revive_uapi::ReturnErrorCode::TransferFailed => CallError::TransferFailed,
            pallet_revive_uapi::ReturnErrorCode::OutOfResources => CallError::OutOfResources,
            _ => CallError::Unknown,
        }
    }
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub fn last_return_data() -> Vec<u8> {
    let len = <api as HostFn>::return_data_size() as usize;
    let mut output = alloc::vec![0u8; len];
    if len > 0 {
        let mut output_ref: &mut [u8] = &mut output[..];
        <api as HostFn>::return_data_copy(&mut output_ref, 0);
    }
    output
}

#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
pub fn last_return_data() -> Vec<u8> {
    panic!("call return data is only available inside contracts");
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub fn call_evm_collect(
    flags: CallFlags,
    callee: &[u8; 20],
    gas: u64,
    value: &[u8; 32],
    input_data: &[u8],
) -> CallResult<Vec<u8>> {
    <api as HostFn>::call_evm(flags, callee, gas, value, input_data, None)
        .map_err(CallError::from)?;
    Ok(last_return_data())
}

#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
pub fn call_evm_collect(
    _flags: CallFlags,
    _callee: &[u8; 20],
    _gas: u64,
    _value: &[u8; 32],
    _input_data: &[u8],
) -> CallResult<Vec<u8>> {
    panic!("cross-contract calls are only available inside contracts");
}
