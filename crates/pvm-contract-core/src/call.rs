use core::{fmt::Debug, marker::PhantomData};

use pallet_revive_uapi::{CallFlags, HostFn, HostFnImpl as api, ReturnErrorCode};
use pvm_contract_types::{Address, SolDecode, SolEncode};
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
/// - uninit # Call was not initialized yet.
pub trait StateMutability: Default + Debug + Clone + Copy {
    fn call_flags(&self) -> CallFlags {
        CallFlags::ALLOW_REENTRY
    }

    fn value(&self) -> u128 {
        0
    }
}

/// Payable stateMutability.
/// CallBuilder with this typeState allows us to set transfer value.
#[derive(Debug, Default, Clone, Copy)]
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
#[derive(Debug, Default, Clone, Copy)]
pub struct NonPayable;
impl StateMutability for NonPayable {}

/// View stateMutability.
/// reads blockchain state.
#[derive(Debug, Default, Clone, Copy)]
pub struct View;
impl StateMutability for View {
    fn call_flags(&self) -> CallFlags {
        CallFlags::ALLOW_REENTRY.union(CallFlags::READ_ONLY)
    }
}

/// Pure stateMutability.
/// this function only operates on it's inputs.
#[derive(Debug, Default, Clone, Copy)]
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
#[derive(Clone, Copy)]
pub struct CallBuilder<Mutability: StateMutability, Inputs: SolEncode, Outputs: SolDecode> {
    pub selector: [u8; 4],
    pub payload: Inputs,
    pub witness: Mutability,
    pub call_limits: CallLimits,
    pub _ret: PhantomData<Outputs>,
}

impl Default for CallBuilder<Pure, (), ()> {
    fn default() -> CallBuilder<Pure, (), ()> {
        Self {
            selector: Default::default(),
            payload: (),
            witness: Pure,
            call_limits: Default::default(),
            _ret: PhantomData,
        }
    }
}

impl<I: SolEncode, R: SolDecode> CallBuilder<Payable, I, R> {
    /// Set the transfer `.value` of the call
    pub fn set_value(mut self, value: u128) -> Self {
        self.witness.value = Some(value);
        self
    }
}

/// so far a temporary function. should be hidden behind a macro call.
pub fn new_payable<Inputs: SolEncode, Ret: SolDecode>(
    selector: [u8; 4],
    data: Inputs,
) -> CallBuilder<Payable, Inputs, Ret> {
    CallBuilder {
        selector,
        payload: data,
        witness: Payable::default(),
        call_limits: Default::default(),
        _ret: Default::default(),
    }
}

/// so far a temporary function. should be hidden behind a macro call.
pub fn new_view<Inputs: SolEncode, Ret: SolDecode>(
    selector: [u8; 4],
    data: Inputs,
) -> CallBuilder<View, Inputs, Ret> {
    CallBuilder {
        selector,
        payload: data,
        witness: View::default(),
        call_limits: Default::default(),
        _ret: Default::default(),
    }
}
/// so far a temporary function. should be hidden behind a macro call.
pub fn new_nonpayable<Inputs: SolEncode, Ret: SolDecode>(
    selector: [u8; 4],
    data: Inputs,
) -> CallBuilder<NonPayable, Inputs, Ret> {
    CallBuilder {
        selector,
        payload: data,
        witness: NonPayable::default(),
        call_limits: Default::default(),
        _ret: Default::default(),
    }
}

impl<Mutability: StateMutability, I: SolEncode, R: SolDecode> CallBuilder<Mutability, I, R> {
    /// Set call limits for the given call
    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
        self.call_limits = limits;
        self
    }

    /// Execute code in the context (storage, caller, value) of the current contract.
    pub fn delegate_call(
        &self,
        address: Address,
        input: &mut [u8],
        output: &mut [u8],
    ) -> Result<R, CallError> {
        let call_flags = CallFlags::empty();
        input[..4].copy_from_slice(&self.selector[..]);
        self.payload.encode_to(&mut input[4..]);
        match self.call_limits {
            CallLimits::GasLimit(limit) => api::delegate_call_evm(
                call_flags,
                &address.0,
                limit,
                &input,
                Some(&mut output.as_mut()),
            ),
            CallLimits::RefTimeAndProofSize {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            } => api::delegate_call(
                call_flags,
                &address.0,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                &input,
                Some(&mut output.as_mut()),
            ),
        }
        .map_err(|error| convert_error(error, &output))
        .map(|_| R::decode(&output))
    }

    /// Call a given contract
    pub fn call(
        &self,
        address: Address,
        input: &mut [u8],
        output: &mut [u8],
    ) -> Result<R, CallError> {
        let call_flags = self.witness.call_flags();
        let value = self.witness.value();
        input[..4].copy_from_slice(&self.selector[..]);
        self.payload.encode_to(&mut input[4..]);
        match self.call_limits {
            CallLimits::GasLimit(limit) => api::call_evm(
                call_flags,
                &address.0,
                limit,
                &U256::from(value).to_be_bytes(),
                &input,
                Some(&mut output.as_mut()),
            ),
            CallLimits::RefTimeAndProofSize {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            } => api::call(
                call_flags,
                &address.0,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                &U256::from(value).to_be_bytes(),
                &input,
                Some(&mut output.as_mut()),
            ),
        }
        .map_err(|error| convert_error(error, &output))
        .map(|_| R::decode(&output))
    }
}

#[cfg(test)]
mod test {
    use core::{default, marker::PhantomData};

    use crate::call::{Pure, StateMutability, View};

    use super::{CallBuilder, NonPayable};

    #[test]
    fn method_available() {
        let builder = CallBuilder {
            selector: [0; 4],
            payload: (),
            witness: super::Payable { value: None },
            call_limits: Default::default(),
            _ret: PhantomData::<()>,
        };

        let _ = builder.set_value(0);
    }

    #[test]
    fn t() {
        struct T<M: StateMutability, const I: bool = false> {
            witness: M,
        }

        impl<M: StateMutability> T<M, true> {
            fn flip(&self) -> T<View, false> {
                T { witness: View }
            }
        }
        impl<M: StateMutability> T<M, false> {
            fn flip(&self) -> T<Pure, true> {
                T { witness: Pure }
            }
        }

        // let b: T<true> = T {
        //     witness: NonPayable,
        // };
        // let c: T<false> = b.flip();
        // let c: T<true> = c.flip();
    }
}
