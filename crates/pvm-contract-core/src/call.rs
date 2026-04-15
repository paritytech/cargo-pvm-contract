use core::{fmt::Debug, marker::PhantomData};

use pallet_revive_uapi::{CallFlags, HostFn, HostFnImpl as api, ReturnErrorCode};
use pvm_contract_types::{
    Address, SolDecode, SolEncode, SolError, const_selector,
    framework_errors::{CALLDATA_TOO_LARGE, INVALID_CALLDATA, NO_SELECTOR, UNKNOWN_SELECTOR},
};
use ruint::aliases::U256;

/// Errors returned by host_api::call()/host_api::instantiate()
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallError {
    /// The called function trapped and has its state changes reverted.
    CalleeTrapped,
    /// Transfer failed for other not further specified reason.
    /// Most probably reserved or locked balance of the sender that was preventing the transfer.
    TransferFailed,
    /// The subcall ran out of weight or storage deposit.
    OutOfResources,
    /// Input buffer too small
    InputBufTooSmall,
    /// Calldata exceeds the fixed buffer size (no-alloc mode only).
    CalldataTooLarge,
    /// Calldata is shorter than the minimum required by the dispatched method.
    InvalidCalldata,
    /// Calldata is shorter than 4 bytes (no selector present).
    NoSelector,
    /// The 4-byte selector does not match any method in the contract.
    UnknownSelector,
    /// The called function ran to completion but decided to revert its state.
    /// Can only be returned from call and instantiate.
    GenericError,
}

impl SolError for CallError {
    const SELECTOR: [u8; 4] = const_selector("CallError(uint256 code)");

    const SIGNATURE: &'static str = "CallError(uint256 code)";

    fn encode_params(&self, buf: &mut [u8]) -> usize {
        match self {
            CallError::CalleeTrapped => {
                let res = U256::from(0);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::TransferFailed => {
                let res = U256::from(1);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::OutOfResources => {
                let res = U256::from(2);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::InputBufTooSmall => {
                let res = U256::from(3);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::CalldataTooLarge => {
                let res = U256::from(4);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::InvalidCalldata => {
                let res = U256::from(5);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::NoSelector => {
                let res = U256::from(6);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::UnknownSelector => {
                let res = U256::from(7);
                res.encode_to(buf);
                return res.encode_len();
            }
            CallError::GenericError => {
                let res = U256::from(7);
                res.encode_to(buf);
                return res.encode_len();
            }
        }
    }

    fn encoded_size(&self) -> usize {
        4 + U256::ZERO.encode_len()
    }
}

fn convert_error(value: ReturnErrorCode, buf: &[u8]) -> CallError {
    match value {
        ReturnErrorCode::CalleeTrapped => CallError::CalleeTrapped,
        ReturnErrorCode::CalleeReverted => {
            if buf.len() >= 4 {
                match &buf[..4] {
                    buf @ _ if buf == &CALLDATA_TOO_LARGE => CallError::CalldataTooLarge,
                    buf @ _ if buf == &INVALID_CALLDATA => CallError::InvalidCalldata,
                    buf @ _ if buf == &NO_SELECTOR => CallError::NoSelector,
                    buf @ _ if buf == &UNKNOWN_SELECTOR => CallError::UnknownSelector,
                    _ => CallError::GenericError,
                }
            } else {
                CallError::GenericError
            }
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

/// Describes call limtis
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RefTimeAndProofSizeLimits {
    /// How much ref_time to devote for the execution. u64::MAX = use all.
    pub ref_time_limit: u64,
    /// How much proof_size to devote for the execution. u64::MAX = use all.
    pub proof_size_limit: u64,
    /// The storage deposit limit for instantiation.
    /// Passing u8::MAX means setting no specific limit for the call, which implies storage usage up to the limit of the parent call.
    pub deposit_limit: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// Describes call limtis
/// default is CallLimits::GasLimit(u64::MAX)
pub enum CallLimits {
    /// Gas limit of the call
    GasLimit(u64),
    /// Native ref_time_limit, proof_time_limit and deposit_limit
    RefTimeAndProofSize(RefTimeAndProofSizeLimits),
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
        input_buf: &mut [u8],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        if input_buf.len() < 4 + self.payload.encode_len() {
            return Err(CallError::InputBufTooSmall);
        }
        let call_flags = CallFlags::empty();
        input_buf[..4].copy_from_slice(&self.selector[..]);
        self.payload.encode_to(&mut input_buf[4..]);
        match self.call_limits {
            CallLimits::GasLimit(limit) => api::delegate_call_evm(
                call_flags,
                &address.0,
                limit,
                &input_buf,
                Some(&mut output_buf.as_mut()),
            ),
            CallLimits::RefTimeAndProofSize(RefTimeAndProofSizeLimits {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            }) => api::delegate_call(
                call_flags,
                &address.0,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                &input_buf,
                Some(&mut output_buf.as_mut()),
            ),
        }
        .map_err(|error| convert_error(error, &output_buf))
        .map(|_| R::decode(&output_buf))
    }

    /// Call a given contract
    pub fn instantiate(
        &self,
        limits: RefTimeAndProofSizeLimits,
        value: u128,
        code_hash: &[u8; 32],
        salt: Option<&[u8; 32]>,
        input_buf: &mut [u8],
        address_buf: &mut [u8; 20],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        if input_buf.len() < 32 + self.payload.encode_len() {
            return Err(CallError::InputBufTooSmall);
        }
        input_buf[..32].copy_from_slice(&code_hash[..]);
        self.payload.encode_to(&mut input_buf[32..]);
        api::instantiate(
            limits.ref_time_limit,
            limits.proof_size_limit,
            &limits.deposit_limit,
            &U256::from(value).to_be_bytes(),
            &input_buf,
            Some(address_buf),
            Some(&mut output_buf.as_mut()),
            salt,
        )
        .map_err(|error| convert_error(error, &output_buf))
        .map(|_| R::decode(&output_buf))
    }

    /// Call a given contract
    pub fn call(
        &self,
        address: Address,
        input_buf: &mut [u8],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        if input_buf.len() < 4 + self.payload.encode_len() {
            return Err(CallError::InputBufTooSmall);
        }
        let call_flags = self.witness.call_flags();
        let value = self.witness.value();
        input_buf[..4].copy_from_slice(&self.selector[..]);
        self.payload.encode_to(&mut input_buf[4..]);
        match self.call_limits {
            CallLimits::GasLimit(limit) => api::call_evm(
                call_flags,
                &address.0,
                limit,
                &U256::from(value).to_be_bytes(),
                &input_buf,
                Some(&mut output_buf.as_mut()),
            ),
            CallLimits::RefTimeAndProofSize(RefTimeAndProofSizeLimits {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            }) => api::call(
                call_flags,
                &address.0,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                &U256::from(value).to_be_bytes(),
                &input_buf,
                Some(&mut output_buf.as_mut()),
            ),
        }
        .map_err(|error| convert_error(error, &output_buf))
        .map(|_| R::decode(&output_buf))
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
}
