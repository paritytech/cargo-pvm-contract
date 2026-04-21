//! The outcome of running a contract's dispatch function.
//!
//! The macro-generated `__pvm_dispatch_call` / `__pvm_dispatch_deploy` and the
//! DSL's `ContractBuilder::dispatch_impl` all return a [`DispatchOutcome`] —
//! they do not diverge, do not touch `return_value`, and are fully testable
//! with plain `Result` assertions.
//!
//! Production riscv64 `call()` / `deploy()` wrappers call [`finalize`] at the
//! entry-point boundary, which maps the outcome to
//! `pallet_revive_uapi::HostFnImpl::return_value`. [`finalize`] is the **only**
//! `-> !` symbol in the testing-reachable surface, and is gated so host test
//! builds cannot accidentally link to it.

use alloc::vec::Vec;

use crate::ReturnFlags;

/// The encoded outcome of a dispatch call.
///
/// `Ok` — success, `data` is the ABI-encoded return payload.
/// `Revert` — revert, `data` is the ABI-encoded revert payload (selector + fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    Ok(Vec<u8>),
    Revert(Vec<u8>),
}

impl DispatchOutcome {
    /// The flags that would be passed to `HostFnImpl::return_value` for this outcome.
    pub fn flags(&self) -> ReturnFlags {
        match self {
            Self::Ok(_) => ReturnFlags::empty(),
            Self::Revert(_) => ReturnFlags::REVERT,
        }
    }

    /// The payload bytes (regardless of Ok/Revert).
    pub fn data(&self) -> &[u8] {
        match self {
            Self::Ok(d) | Self::Revert(d) => d,
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }

    pub fn is_revert(&self) -> bool {
        matches!(self, Self::Revert(_))
    }
}

/// Terminate the contract by handing the outcome to the runtime.
///
/// **riscv64-only.** This is the *only* place contract code hits
/// `HostFnImpl::return_value`, and it is the *only* diverging function in the
/// testing surface. Host-target test builds cannot reach this symbol — it is
/// `#[cfg(target_arch = "riscv64")]` gated at the module level.
#[cfg(target_arch = "riscv64")]
pub fn finalize(outcome: DispatchOutcome) -> ! {
    use pallet_revive_uapi::HostFn as _;
    match outcome {
        DispatchOutcome::Ok(data) => {
            pallet_revive_uapi::HostFnImpl::return_value(ReturnFlags::empty(), &data)
        }
        DispatchOutcome::Revert(data) => {
            pallet_revive_uapi::HostFnImpl::return_value(ReturnFlags::REVERT, &data)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn ok_and_revert_flags() {
        let ok = DispatchOutcome::Ok(vec![1, 2, 3]);
        let rv = DispatchOutcome::Revert(vec![4, 5]);
        assert_eq!(ok.flags(), ReturnFlags::empty());
        assert_eq!(rv.flags(), ReturnFlags::REVERT);
        assert!(ok.is_ok());
        assert!(rv.is_revert());
        assert_eq!(ok.data(), &[1, 2, 3]);
        assert_eq!(rv.data(), &[4, 5]);
    }
}
