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

// ---------------------------------------------------------------------------
// CDM cross-contract compatibility surface
//
// These items let contracts on this (`charles/cdm-integration`) branch consume
// the newer `cdm::import!` / `pvm_cdm::reference!` macros from the
// contract-dependency-manager `cdm` crate. Those macros emit a 4-generic
// contract type and resolve addresses via a `PolkaVmHost: HostApi` call. We
// provide thin, ambient-host implementations so the generated code type-checks
// and runs against this branch's `api::call_evm` syscall — without pulling in
// the `sm/cdm` typestate/CallBuilder machinery (the imported methods still go
// through this branch's existing direct-call codegen).
// ---------------------------------------------------------------------------

/// Type-state marker used by `cdm::import!`-generated contract types. The
/// imported cross-contract methods on this branch call the ambient host
/// directly, so the marker carries no behavior — it exists only to match the
/// generic arity `pvm_cdm::reference!` expects (`Foo<Pure, (), (), false>`).
pub struct Pure;

/// Ambient PolkaVM host handle. Zero-sized; routes to the `pallet_revive`
/// syscalls exposed via [`crate::api`].
pub struct PolkaVmHost;

/// Minimal host surface required by `pvm_cdm::reference!`'s generated
/// `cdm_lookup()` (a raw `getAddress(string)` registry call). The signature
/// mirrors `pallet_revive_uapi::HostFn::call_evm`.
pub trait HostApi {
    fn call_evm(
        &self,
        flags: crate::CallFlags,
        address: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> Result<(), pallet_revive_uapi::ReturnErrorCode>;
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
impl HostApi for PolkaVmHost {
    fn call_evm(
        &self,
        flags: crate::CallFlags,
        address: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> Result<(), pallet_revive_uapi::ReturnErrorCode> {
        <crate::api as crate::HostFn>::call_evm(flags, address, gas, value, data, output)
    }
}

// Off-target (host) stub so the type is nameable in tooling builds. Cross-contract
// calls only execute on the PolkaVM target, so this is never reached at runtime.
#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
impl HostApi for PolkaVmHost {
    fn call_evm(
        &self,
        _flags: crate::CallFlags,
        _address: &[u8; 20],
        _gas: u64,
        _value: &[u8; 32],
        _data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> Result<(), pallet_revive_uapi::ReturnErrorCode> {
        unreachable!("HostApi::call_evm is only available on the PolkaVM target")
    }
}
