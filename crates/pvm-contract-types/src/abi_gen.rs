extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// A parameter in a Solidity ABI function signature.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AbiParam {
    /// Parameter name (empty string for unnamed outputs).
    pub name: String,
    /// Solidity type name (e.g. "uint256", "address", "tuple").
    #[serde(rename = "type")]
    pub param_type: String,
    /// For tuple types, the list of sub-parameters. Empty for primitives.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<AbiParam>,
}

/// A top-level item in a Solidity ABI JSON array.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AbiItem {
    Constructor {
        inputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        state_mutability: String,
    },
    Function {
        name: String,
        inputs: Vec<AbiParam>,
        outputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        state_mutability: String,
    },
}

/// Serialize a list of ABI items to a JSON string.
pub fn abi_to_json(items: &[AbiItem]) -> String {
    serde_json::to_string(items).expect("ABI serialization failed")
}
