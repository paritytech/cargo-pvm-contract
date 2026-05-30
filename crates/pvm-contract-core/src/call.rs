use core::{fmt::Debug, marker::PhantomData};

use pvm_contract_types::{
    Address, CallFlags, ContractContext, DecodeError, Host, HostApi, ReturnErrorCode, SolDecode,
    SolEncode, SolError, const_selector,
};
use ruint::aliases::U256;

#[cfg(feature = "alloc")]
extern crate alloc;

/// Errors returned by `HostApi::call()` / `HostApi::instantiate()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum CallError {
    /// The called function trapped and has its state changes reverted.
    CalleeTrapped = 0,
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
    /// Payload decoding error
    DecodingError,
    /// Unknown error occured
    Unknown = 8,
}

impl CallError {
    /// Try and decode error using a provided `buf` buffer for the error data
    /// Note: this will try to decode an error only if the `CallError::CalleeReverted` variant is reached,
    /// otherwise `Ok(None)` will be returned.
    pub fn try_decode_error_raw<T: SolError>(
        &self,
        host: &Host,
        mut buf: &mut [u8],
    ) -> Result<Option<T>, DecodeError> {
        match self {
            Self::CalleeReverted => {
                let size = host.return_data_size();
                if buf.len() < size as usize {
                    return Err(DecodeError);
                }
                host.return_data_copy(&mut buf, 0);
                T::decode_at(buf, 0)
            }
            _ => Ok(None),
        }
    }

    #[cfg(feature = "alloc")]
    /// Try and decode error.
    /// Note: this will try to decode an error only if the `CallError::CalleeReverted` variant is reached,
    /// otherwise `Ok(None)` will be returned.
    ///
    /// # Safety:
    /// - buffer for error returns is unlimited: check beforehand with `host.return_data_size()`
    /// - decoding only works if error is decoded immediately after making the cross-contract call,
    ///   otherwise the buffer containing the error is overwritten by the runtime
    pub fn try_decode_error<T: SolError>(&self, host: &Host) -> Result<Option<T>, DecodeError> {
        match self {
            Self::CalleeReverted => {
                let size = host.return_data_size();
                let mut buf = alloc::vec![0; size as usize];
                host.return_data_copy(&mut buf.as_mut_slice(), 0);
                T::decode_at(&buf, 0)
            }
            _ => Ok(None),
        }
    }
}

impl From<DecodeError> for CallError {
    fn from(_: DecodeError) -> Self {
        Self::DecodingError
    }
}

impl CallError {
    fn discriminant(&self) -> u8 {
        *self as u8
    }
}

impl SolError for CallError {
    const SELECTOR: [u8; 4] = const_selector("CallError(uint256)");

    const SIGNATURE: &'static str = "CallError(uint256)";

    fn encoded_size(&self) -> usize {
        36
    }

    fn encode_to(&self, buf: &mut [u8]) -> usize {
        buf[0..4].copy_from_slice(&Self::SELECTOR);
        let res = U256::from(self.discriminant());
        res.encode_to(&mut buf[4..]);
        res.encode_len() + 4
    }

    fn decode_at(input: &[u8], offset: usize) -> Result<Option<Self>, DecodeError> {
        if input.len() < 4 {
            return Err(DecodeError);
        }
        if input
            .get(offset..offset + 4)
            .is_some_and(|x| x == Self::SELECTOR)
        {
            let data = u8::decode_at(input, offset + 4)?;
            match data {
                0 => Ok(Some(Self::CalleeTrapped)),
                1 => Ok(Some(Self::TransferFailed)),
                2 => Ok(Some(Self::OutOfResources)),
                3 => Ok(Some(Self::DuplicateContractAddress)),
                4 => Ok(Some(Self::InputBufTooSmall)),
                5 => Ok(Some(Self::OutputBufTooSmall)),
                6 => Ok(Some(Self::CalleeReverted)),
                7 => Ok(Some(Self::DecodingError)),
                8 => Ok(Some(Self::Unknown)),
                _ => Err(DecodeError),
            }
        } else {
            Ok(None)
        }
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
///
/// ## Reentrancy
///
/// By default, pallet-revive rejects reentrant calls. If contract A calls
/// contract B, B (and its callees) cannot call back into A. To opt in to
/// reentrancy, use [`allow_reentry()`](CallBuilder::allow_reentry):
///
/// ```ignore
/// let result = foo.bar()
///     .allow_reentry()
///     .call(self.host())?;
/// ```
///
/// Only enable this when your contract is designed to handle callbacks
/// (e.g., flash loans, ERC-777 hooks). The runtime protection exists to
/// prevent the classic reentrancy attack where a callee re-enters the
/// caller before state updates are complete.
#[derive(Clone, Copy)]
pub struct CallBuilder<Mutability: StateMutability, Inputs: SolEncode, Outputs: SolDecode> {
    pub selector: [u8; 4],
    pub payload: Inputs,
    pub witness: Mutability,
    pub call_limits: CallLimits,
    pub allow_reentry: bool,
    pub _ret: PhantomData<Outputs>,
}

impl Default for CallBuilder<Pure, (), ()> {
    fn default() -> CallBuilder<Pure, (), ()> {
        Self {
            selector: Default::default(),
            payload: (),
            witness: Pure,
            call_limits: Default::default(),
            allow_reentry: false,
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
    /// Set call limits for the given call.
    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
        self.call_limits = limits;
        self
    }

    /// Allow the callee to re-enter this contract.
    ///
    /// Without this, pallet-revive rejects any call from the callee (or its
    /// callees) back into the current contract. Enable only when the contract
    /// is designed to handle callbacks safely.
    ///
    /// Applies to `call` and `call_raw` only. Delegate calls ignore this
    /// flag because pallet-revive requires `ALLOW_REENTRY` to be unset for
    /// delegate calls and returns `InvalidCallFlags` otherwise.
    pub fn allow_reentry(mut self) -> Self {
        self.allow_reentry = true;
        self
    }

    /// Decode the most recent call's return data into `R`.
    ///
    /// Read-only — does not mutate state, so it is intentionally not gated by
    /// [`ContractContext`]. Internal helper used by `call` / `delegate_call` /
    /// `instantiate`; safe to call from `&self` methods directly when reading
    /// return data after a manual `*_raw` call.
    pub fn extract_output(&self, host: &Host, mut output_buf: &mut [u8]) -> Result<R, CallError> {
        if self.output_size(host) > output_buf.len() {
            return Err(CallError::OutputBufTooSmall);
        }
        host.return_data_copy(&mut output_buf, 0);
        R::decode(output_buf).map_err(Into::into)
    }

    pub fn output_size(&self, host: &Host) -> usize {
        // safe as we always run on 64bit arches
        host.return_data_size() as usize
    }

    /// Internal — actually invoke the cross-contract call. Mutability gating
    /// happens at the public surface (`call_raw` / `call` per typestate).
    fn call_raw_inner(
        &self,
        host: &Host,
        address: Address,
        input_buf: &mut [u8],
    ) -> Result<(), CallError> {
        if input_buf.len() < 4 + self.payload.encode_len() {
            return Err(CallError::InputBufTooSmall);
        }
        let call_flags = if self.allow_reentry {
            self.witness.call_flags() | CallFlags::ALLOW_REENTRY
        } else {
            self.witness.call_flags()
        };
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

    /// Internal — actually invoke the delegate call. Always mutating from the
    /// caller's perspective: callee runs in caller's storage context, so any
    /// callee write hits caller's storage. Gated `&mut impl ContractContext` at
    /// the public surface regardless of the callee's declared mutability.
    fn delegate_call_raw_inner(
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

    /// Internal — actually invoke instantiate. Always mutating: transfers
    /// value, emits a deploy event, bumps the caller's nonce.
    #[allow(clippy::too_many_arguments)]
    fn instantiate_raw_inner(
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

    /// Delegate-call another contract.
    ///
    /// Always requires `&mut impl ContractContext`: the callee runs in caller's
    /// storage context, so even a `View`-typestate delegate call could mutate
    /// caller's state. Borrow gate is on `Self`, not the callee mutability.
    pub fn delegate_call_raw<R0: ContractContext>(
        &self,
        root: &mut R0,
        address: Address,
        input_buf: &mut [u8],
    ) -> Result<(), CallError> {
        self.delegate_call_raw_inner(root.host(), address, input_buf)
    }

    /// Delegate-call another contract and decode the output.
    pub fn delegate_call<R0: ContractContext>(
        &self,
        root: &mut R0,
        address: Address,
        input_buf: &mut [u8],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        // Clone the host handle before the mutable borrow of `root`: on
        // `riscv64` `Host` is a ZST (free `Copy`), on host-target builds it's
        // a refcount bump on `Rc<dyn HostApi>`. Avoids re-borrowing `root`
        // after the mutable call to read return data.
        let host = root.host().clone();
        self.delegate_call_raw(root, address, input_buf)?;
        self.extract_output(&host, output_buf)
    }

    /// Instantiate a new contract.
    ///
    /// Always requires `&mut impl ContractContext`: instantiation transfers
    /// value, emits a deploy event, and bumps the caller's nonce.
    #[allow(clippy::too_many_arguments)]
    pub fn instantiate_raw<R0: ContractContext>(
        &self,
        root: &mut R0,
        limits: RefTimeAndProofSizeLimits,
        value: u128,
        code_hash: &[u8; 32],
        salt: Option<&[u8; 32]>,
        address_buf: &mut [u8; 20],
        input_buf: &mut [u8],
    ) -> Result<(), CallError> {
        self.instantiate_raw_inner(
            root.host(),
            limits,
            value,
            code_hash,
            salt,
            address_buf,
            input_buf,
        )
    }

    /// Instantiate a new contract and decode the constructor's output.
    #[allow(clippy::too_many_arguments)]
    pub fn instantiate<R0: ContractContext>(
        &self,
        root: &mut R0,
        limits: RefTimeAndProofSizeLimits,
        value: u128,
        code_hash: &[u8; 32],
        salt: Option<&[u8; 32]>,
        address_buf: &mut [u8; 20],
        input_buf: &mut [u8],
        output_buf: &mut [u8],
    ) -> Result<R, CallError> {
        // Clone first — see `delegate_call` for rationale.
        let host = root.host().clone();
        self.instantiate_raw(root, limits, value, code_hash, salt, address_buf, input_buf)?;
        self.extract_output(&host, output_buf)
    }
}

// ---------------------------------------------------------------------------
// View / Pure callees: read-only, callable from `&self` methods.
// ---------------------------------------------------------------------------

macro_rules! impl_readonly_call {
    ($mutability:ident) => {
        impl<I: SolEncode, R: SolDecode> CallBuilder<$mutability, I, R> {
            /// Call a `view`/`pure` callee. Borrows the contract root
            /// immutably, so this is callable from `&self` methods.
            pub fn call_raw<R0: ContractContext>(
                &self,
                root: &R0,
                address: Address,
                input_buf: &mut [u8],
            ) -> Result<(), CallError> {
                self.call_raw_inner(root.host(), address, input_buf)
            }

            /// Call a `view`/`pure` callee and decode its output.
            pub fn call<R0: ContractContext>(
                &self,
                root: &R0,
                address: Address,
                input_buf: &mut [u8],
                output_buf: &mut [u8],
            ) -> Result<R, CallError> {
                let host = root.host();
                self.call_raw_inner(host, address, input_buf)?;
                self.extract_output(host, output_buf)
            }
        }
    };
}
impl_readonly_call!(View);
impl_readonly_call!(Pure);

// ---------------------------------------------------------------------------
// NonPayable / Payable callees: state-mutating, require `&mut self`.
// ---------------------------------------------------------------------------

macro_rules! impl_mutating_call {
    ($mutability:ident) => {
        impl<I: SolEncode, R: SolDecode> CallBuilder<$mutability, I, R> {
            /// Call a `nonpayable`/`payable` callee. Borrows the contract
            /// root mutably, so this is only callable from `&mut self`
            /// methods. A `&self` (view) method cannot construct the
            /// `&mut impl ContractContext` required here, so the borrow checker
            /// rejects view methods that try to initiate a state-mutating
            /// cross-contract call.
            pub fn call_raw<R0: ContractContext>(
                &self,
                root: &mut R0,
                address: Address,
                input_buf: &mut [u8],
            ) -> Result<(), CallError> {
                self.call_raw_inner(root.host(), address, input_buf)
            }

            /// Call a `nonpayable`/`payable` callee and decode its output.
            pub fn call<R0: ContractContext>(
                &self,
                root: &mut R0,
                address: Address,
                input_buf: &mut [u8],
                output_buf: &mut [u8],
            ) -> Result<R, CallError> {
                // Clone first — see `delegate_call` for rationale.
                let host = root.host().clone();
                self.call_raw(root, address, input_buf)?;
                self.extract_output(&host, output_buf)
            }
        }
    };
}
impl_mutating_call!(NonPayable);
impl_mutating_call!(Payable);

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
            allow_reentry: false,
            _ret: PhantomData::<()>,
        };

        let _ = builder.set_value(0);
    }
}
