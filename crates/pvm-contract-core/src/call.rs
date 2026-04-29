use core::{fmt::Debug, marker::PhantomData};

use pvm_contract_types::{
    Address, CallFlags, Host, HostApi, ReturnErrorCode, SolDecode, SolEncode, SolError,
    const_selector,
};
use ruint::aliases::U256;

/// Errors returned by `HostApi::call()` / `HostApi::instantiate()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum CallError {
    /// The called function trapped and has its state changes reverted.
    CalleeTrapped,
    /// Transfer failed for other not further specified reason.
    /// Most probably reserved or locked balance of the sender that was preventing the transfer.
    TransferFailed,
    /// The subcall ran out of weight or storage deposit.
    OutOfResources,
    /// Contract instantiation failed because the address already exists.
    /// Occurs when instantiating the same contract with the same salt more than once.
    DuplicateContractAddress,
    /// Input buffer too small
    InputBufTooSmall,
    /// Output buffer too small
    OutputBufTooSmall,
    /// The called function ran to completion but decided to revert its state.
    /// Can only be returned from call and instantiate.
    CalleeReverted,
    /// Unknown error occured
    Unknown,
}

impl SolError for CallError {
    const SELECTOR: [u8; 4] = const_selector("CallError(uint256)");

    const SIGNATURE: &'static str = "CallError(uint256)";

    fn encode_params(&self, buf: &mut [u8]) -> usize {
        let res = U256::from(*self as u8);
        res.encode_to(buf);
        res.encode_len()
    }

    fn encoded_size(&self) -> usize {
        36
    }
}

fn convert_error(value: ReturnErrorCode) -> CallError {
    match value {
        ReturnErrorCode::CalleeTrapped => CallError::CalleeTrapped,
        ReturnErrorCode::CalleeReverted => CallError::CalleeReverted,
        ReturnErrorCode::TransferFailed => CallError::TransferFailed,
        ReturnErrorCode::OutOfResources => CallError::OutOfResources,
        ReturnErrorCode::DuplicateContractAddress => CallError::DuplicateContractAddress,
        _ => CallError::Unknown,
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
        CallFlags::empty()
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
        CallFlags::READ_ONLY
    }
}

/// Pure stateMutability.
/// this function only operates on it's inputs.
#[derive(Debug, Default, Clone, Copy)]
pub struct Pure;
impl StateMutability for Pure {
    fn call_flags(&self) -> CallFlags {
        CallFlags::READ_ONLY
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
    pub fn delegate_call_raw(
        &self,
        host: &Host,
        address: Address,
        input_buf: &mut [u8],
    ) -> Result<(), CallError> {
        if input_buf.len() < 4 + self.payload.encode_len() {
            return Err(CallError::InputBufTooSmall);
        }
        let call_flags = CallFlags::empty();
        input_buf[..4].copy_from_slice(&self.selector[..]);
        self.payload.encode_to(&mut input_buf[4..]);
        match self.call_limits {
            CallLimits::GasLimit(limit) => {
                host.delegate_call_evm(call_flags, &address.0, limit, input_buf, None)
            }
            CallLimits::RefTimeAndProofSize(RefTimeAndProofSizeLimits {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            }) => host.delegate_call(
                call_flags,
                &address.0,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                input_buf,
                None,
            ),
        }
        .map_err(convert_error)
    }

    pub fn extract_output(&self, host: &Host, mut output_buf: &mut [u8]) -> Result<R, CallError> {
        if self.output_size(host) > output_buf.len() {
            return Err(CallError::OutputBufTooSmall);
        }
        host.return_data_copy(&mut output_buf, 0);
        Ok(R::decode(output_buf))
    }

    pub fn output_size(&self, host: &Host) -> usize {
        // safe as we always run on 64bit arches
        host.return_data_size() as usize
    }

    /// Execute code in the context (storage, caller, value) of the current contract.
    pub fn delegate_call(
        &self,
        host: &Host,
        address: Address,
        input_buf: &mut [u8],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        self.delegate_call_raw(host, address, input_buf)
            .and_then(|_| self.extract_output(host, output_buf))
    }

    /// Call a given contract
    #[allow(clippy::too_many_arguments)]
    pub fn instantiate_raw(
        &self,
        host: &Host,
        limits: RefTimeAndProofSizeLimits,
        value: u128,
        code_hash: &[u8; 32],
        salt: Option<&[u8; 32]>,
        address_buf: &mut [u8; 20],
        input_buf: &mut [u8],
    ) -> Result<(), CallError> {
        if input_buf.len() < 32 + self.payload.encode_len() {
            return Err(CallError::InputBufTooSmall);
        }
        input_buf[..32].copy_from_slice(&code_hash[..]);
        self.payload.encode_to(&mut input_buf[32..]);
        host.instantiate(
            limits.ref_time_limit,
            limits.proof_size_limit,
            &limits.deposit_limit,
            &U256::from(value).to_be_bytes(),
            input_buf,
            Some(address_buf),
            None,
            salt,
        )
        .map_err(convert_error)
    }

    #[allow(clippy::too_many_arguments)]
    /// Execute code in the context (storage, caller, value) of the current contract.
    pub fn instantiate(
        &self,
        host: &Host,
        limits: RefTimeAndProofSizeLimits,
        value: u128,
        code_hash: &[u8; 32],
        salt: Option<&[u8; 32]>,
        address_buf: &mut [u8; 20],
        input_buf: &mut [u8],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        self.instantiate_raw(host, limits, value, code_hash, salt, address_buf, input_buf)
            .and_then(|_| self.extract_output(host, output_buf))
    }
    /// Call a given contract
    pub fn call_raw(
        &self,
        host: &Host,
        address: Address,
        input_buf: &mut [u8],
    ) -> Result<(), CallError> {
        if input_buf.len() < 4 + self.payload.encode_len() {
            return Err(CallError::InputBufTooSmall);
        }
        let call_flags = self.witness.call_flags();
        let value = self.witness.value();
        input_buf[..4].copy_from_slice(&self.selector[..]);
        self.payload.encode_to(&mut input_buf[4..]);
        match self.call_limits {
            CallLimits::GasLimit(limit) => host.call_evm(
                call_flags,
                &address.0,
                limit,
                &U256::from(value).to_be_bytes(),
                input_buf,
                None,
            ),
            CallLimits::RefTimeAndProofSize(RefTimeAndProofSizeLimits {
                ref_time_limit,
                proof_size_limit,
                deposit_limit,
            }) => host.call(
                call_flags,
                &address.0,
                ref_time_limit,
                proof_size_limit,
                &deposit_limit,
                &U256::from(value).to_be_bytes(),
                input_buf,
                None,
            ),
        }
        .map_err(convert_error)
    }

    /// Execute code in the context (storage, caller, value) of the current contract.
    pub fn call(
        &self,
        host: &Host,
        address: Address,
        input_buf: &mut [u8],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        self.call_raw(host, address, input_buf)
            .and_then(|_| self.extract_output(host, output_buf))
    }
}

#[cfg(test)]
mod test {
    use core::marker::PhantomData;

    use super::CallBuilder;

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
