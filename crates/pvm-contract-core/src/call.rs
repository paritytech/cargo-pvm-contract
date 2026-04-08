use core::fmt::Debug;

use pallet_revive_uapi::{CallFlags, HostFn, HostFnImpl as api, ReturnErrorCode};
use pvm_contract_types::{Address, SolDecode};
use ruint::aliases::U256;

/// Errors returned by host_api::call()
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallError {
    /// The called function ran to completion but decided to revert its state.
    /// Can only be returned from call and instantiate.
    CalleeReverted([u8; 512]),
    /// The called function trapped and has its state changes reverted.
    CalleeTrapped,
    /// Transfer failed for other not further specified reason.
    /// Most probably reserved or locked balance of the sender that was preventing the transfer.
    TransferFailed,
    /// The subcall ran out of weight or storage deposit.
    OutOfResources,
}

impl AsRef<[u8]> for CallError {
    fn as_ref(&self) -> &[u8] {
        match *self {
            _ => b"contract call error",
        }
    }
}

fn convert_error(value: ReturnErrorCode, buf: &[u8]) -> CallError {
    match value {
        ReturnErrorCode::CalleeTrapped => CallError::CalleeTrapped,
        ReturnErrorCode::CalleeReverted => {
            let mut slice = [0; 512];
            slice.copy_from_slice(buf);
            CallError::CalleeReverted(slice)
        }
        ReturnErrorCode::TransferFailed => CallError::TransferFailed,
        ReturnErrorCode::OutOfResources => CallError::OutOfResources,
        _ => panic!("shouldn't happen"),
    }
}

/// StateMutability of a given function
/// can be one of:
/// - view
/// - pure
/// - nonpayable # this is the default stateMutability
/// - payable
pub trait StateMutability: Default + Debug {
    fn call_flags(&self) -> CallFlags {
        CallFlags::ALLOW_REENTRY
    }

    fn value(&self) -> u128 {
        0
    }
}

/// Payable stateMutability.
/// CallBuilder with this typeState allows us to set transfer value.
#[derive(Debug, Default)]
pub struct Payable {
    value: Option<u128>,
}
impl StateMutability for Payable {
    fn value(&self) -> u128 {
        self.value.unwrap_or_default()
    }
}

/// NonPayable stateMutability.
/// StateMutability selected by default.
#[derive(Debug, Default)]
pub struct NonPayable;
impl StateMutability for NonPayable {}

/// View stateMutability.
/// reads blockchain state.
#[derive(Debug, Default)]
pub struct View;
impl StateMutability for View {
    fn call_flags(&self) -> CallFlags {
        CallFlags::ALLOW_REENTRY.union(CallFlags::READ_ONLY)
    }
}

/// Pure stateMutability.
/// this function only operates on it's inputs.
#[derive(Debug, Default)]
pub struct Pure;
impl StateMutability for Pure {
    fn call_flags(&self) -> CallFlags {
        CallFlags::ALLOW_REENTRY.union(CallFlags::READ_ONLY)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// Describes call limtis
/// default is CallLimits::GasLimit(u64::MAX)
pub enum CallLimits {
    /// Gas limit of the call
    GasLimit(u64),
    /// Native ref_time_limit, proof_time_limit and deposit_limit
    RefTimeAndProofSize {
        /// How much ref_time to devote for the execution. u64::MAX = use all.
        ref_time_limit: u64,
        /// How much proof_size to devote for the execution. u64::MAX = use all.
        proof_size_limit: u64,
        /// The storage deposit limit for instantiation.
        /// Passing u8::MAX means setting no specific limit for the call, which implies storage usage up to the limit of the parent call.
        deposit_limit: [u8; 32],
    },
}

impl Default for CallLimits {
    fn default() -> Self {
        CallLimits::GasLimit(u64::MAX)
    }
}

/// Call builder to construct and configure calls.
/// depending on the [StateMutability] param can have additional methods.
pub struct CallBuilder<'a, Mutability: StateMutability> {
    address: [u8; 20],
    payload: &'a [u8],
    witness: Mutability,
    call_limits: CallLimits,
}

impl<'a> CallBuilder<'a, Payable> {
    /// Set the transfer `.value` of the call
    pub fn set_value(mut self, value: u128) -> Self {
        self.witness.value = Some(value);
        self
    }
}

/// so far a temporary function. should be hidden behind a macro call.
pub fn new_payable<'a>(address: Address, data: &'a [u8]) -> CallBuilder<'a, Payable> {
    CallBuilder {
        address: address.0,
        payload: data,
        witness: Payable::default(),
        call_limits: Default::default(),
    }
}

/// so far a temporary function. should be hidden behind a macro call.
pub fn new_view<'a>(address: Address, data: &'a [u8]) -> CallBuilder<'a, View> {
    CallBuilder {
        address: address.0,
        payload: &data,
        witness: View::default(),
        call_limits: Default::default(),
    }
}

impl<'a, Mutability: StateMutability> CallBuilder<'a, Mutability> {
    /// Set call limits for the given call
    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
        self.call_limits = limits;
        self
    }

    /// Execute code in the context (storage, caller, value) of the current contract.
    pub fn delegate_call<T: SolDecode>(&self) -> Result<T, CallError> {
        let call_flags = CallFlags::empty();
        let mut buf = [0; 512];
        match self.call_limits {
            CallLimits::GasLimit(limit) => api::delegate_call_evm(
                call_flags,
                &self.address,
                limit,
                &self.payload,
                Some(&mut buf.as_mut_slice()),
            ),
            CallLimits::RefTimeAndProofSize {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            } => api::delegate_call(
                call_flags,
                &self.address,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                &self.payload,
                Some(&mut buf.as_mut_slice()),
            ),
        }
        .map_err(|error| convert_error(error, &buf))
        .map(|_| T::decode(&buf))
    }

    /// Call a given contract
    pub fn call<T: SolDecode>(&self) -> Result<T, CallError> {
        let call_flags = self.witness.call_flags();
        let value = self.witness.value();
        let mut buf = [0; 512];
        match self.call_limits {
            CallLimits::GasLimit(limit) => api::call_evm(
                call_flags,
                &self.address,
                limit,
                &U256::from(value).to_be_bytes(),
                &self.payload,
                Some(&mut buf.as_mut_slice()),
            ),
            CallLimits::RefTimeAndProofSize {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            } => api::call(
                call_flags,
                &self.address,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                &U256::from(value).to_be_bytes(),
                &self.payload,
                Some(&mut buf.as_mut_slice()),
            ),
        }
        .map_err(|error| convert_error(error, &buf))
        .map(|_| T::decode(&buf))
    }
}

#[cfg(test)]
mod test {
    use std::marker::PhantomData;

    use super::{CallBuilder, NonPayable};

    #[test]
    fn method_available() {
        let builder = CallBuilder {
            address: [0; 20],
            payload: &[0u8; 32],
            witness: super::Payable { value: None },
            call_limits: Default::default(),
        };

        let _ = builder.set_value(0);
    }
}
