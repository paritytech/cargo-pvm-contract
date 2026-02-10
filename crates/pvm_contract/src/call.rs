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
