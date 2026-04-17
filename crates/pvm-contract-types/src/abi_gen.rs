extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// A parameter in a Solidity ABI function signature.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AbiParam {
    /// Parameter name (empty string for unnamed outputs).
    pub name: String,
    /// Solidity type name (e.g. "uint256", "address", "tuple").
    #[serde(rename = "type")]
    pub param_type: String,
    /// For tuple types, the list of sub-parameters. Empty for primitives.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<AbiParam>,
}

/// A top-level item in a Solidity ABI JSON array.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AbiItem {
    Constructor {
        inputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        state_mutability: Option<String>,
    },
    Function {
        name: String,
        inputs: Vec<AbiParam>,
        outputs: Vec<AbiParam>,
        #[serde(rename = "stateMutability")]
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        state_mutability: Option<String>,
    },
    Error {
        name: String,
        inputs: Vec<AbiParam>,
    },
}

/// Wrapper for a complete ABI JSON array.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AbiJson(pub Vec<AbiItem>);

/// Serialize a list of ABI items to a JSON string.
pub fn abi_to_json(items: &[AbiItem]) -> String {
    serde_json::to_string(items).expect("ABI serialization failed")
}
