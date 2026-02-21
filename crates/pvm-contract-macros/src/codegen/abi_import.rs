use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token};

use crate::codegen::decode::generate_decode;
use crate::codegen::encode::generate_encode;
use crate::signature::compute_selector;
use crate::signature::FunctionSignature;
use crate::signature::SolType;
use crate::solidity::to_snake_case;

// ---------------------------------------------------------------------------
// Macro argument parsing
// ---------------------------------------------------------------------------

pub struct AbiImportArgs {
    pub module_name: String,
    pub abi_path: String,
    pub cdm: Option<String>,
}

impl Parse for AbiImportArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let module_name: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        let abi_path: LitStr = input.parse()?;
        let mut cdm = None;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            let ident: Ident = input.parse()?;
            if ident == "cdm" {
                input.parse::<Token![=]>()?;
                let name: LitStr = input.parse()?;
                cdm = Some(name.value());
            }
        }
        Ok(AbiImportArgs {
            module_name: module_name.value(),
            abi_path: abi_path.value(),
            cdm,
        })
    }
}

// ---------------------------------------------------------------------------
// ABI JSON data structures (parsed from serde_json::Value, not via Derive)
// ---------------------------------------------------------------------------

struct AbiParam {
    name: String,
    type_str: String,
    components: Option<Vec<AbiParam>>,
}

struct AbiFunction {
    name: String,
    inputs: Vec<AbiParam>,
    outputs: Vec<AbiParam>,
}

// ---------------------------------------------------------------------------
// ABI JSON parsing
// ---------------------------------------------------------------------------

fn parse_abi_param(val: &serde_json::Value) -> Result<AbiParam, String> {
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let type_str = val
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or("ABI param missing 'type'")?
        .to_string();
    let components = if let Some(arr) = val.get("components").and_then(|v| v.as_array()) {
        let parsed: Result<Vec<_>, _> = arr.iter().map(parse_abi_param).collect();
        Some(parsed?)
    } else {
        None
    };
    Ok(AbiParam {
        name,
        type_str,
        components,
    })
}

fn parse_abi_json(json_str: &str) -> Result<Vec<AbiFunction>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("Invalid ABI JSON: {}", e))?;
    let arr = value
        .as_array()
        .ok_or("ABI JSON must be an array")?;

    let mut functions = Vec::new();
    for entry in arr {
        let ty = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if ty != "function" {
            continue;
        }
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("ABI function missing 'name'")?
            .to_string();
        let inputs = entry
            .get("inputs")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(parse_abi_param).collect::<Result<Vec<_>, _>>())
            .unwrap_or(Ok(vec![]))?;
        let outputs = entry
            .get("outputs")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(parse_abi_param).collect::<Result<Vec<_>, _>>())
            .unwrap_or(Ok(vec![]))?;
        functions.push(AbiFunction {
            name,
            inputs,
            outputs,
        });
    }
    Ok(functions)
}

// ---------------------------------------------------------------------------
// ABI type string -> SolType conversion
// ---------------------------------------------------------------------------

fn parse_abi_type(type_str: &str, components: Option<&[AbiParam]>) -> Result<SolType, String> {
    let type_str = type_str.trim();

    // Handle tuple types (with optional array suffix)
    if type_str == "tuple" || type_str.starts_with("tuple[") {
        let comps = components.ok_or("tuple type requires components")?;
        let inner_types: Result<Vec<SolType>, String> = comps
            .iter()
            .map(|c| parse_abi_type(&c.type_str, c.components.as_deref()))
            .collect();
        let base = SolType::Tuple(inner_types?);

        // Handle tuple[] or tuple[N] suffix
        if type_str == "tuple" {
            return Ok(base);
        }
        let suffix = &type_str["tuple".len()..];
        return apply_array_suffix(base, suffix);
    }

    // Handle array suffix on base types: e.g. "uint256[]" or "address[3]"
    if let Some(bracket_start) = type_str.find('[') {
        let base_str = &type_str[..bracket_start];
        let suffix = &type_str[bracket_start..];
        let base = parse_base_abi_type(base_str)?;
        return apply_array_suffix(base, suffix);
    }

    parse_base_abi_type(type_str)
}

fn parse_base_abi_type(s: &str) -> Result<SolType, String> {
    match s {
        "address" => Ok(SolType::Address),
        "bool" => Ok(SolType::Bool),
        "string" => Ok(SolType::String),
        "bytes" => Ok(SolType::DynBytes),
        _ if s.starts_with("uint") => {
            let bits: usize = s[4..].parse().unwrap_or(256);
            if bits == 0 || bits > 256 || bits % 8 != 0 {
                return Err(format!("Invalid uint size: {}", bits));
            }
            Ok(SolType::Uint(bits))
        }
        _ if s.starts_with("int") => {
            let bits: usize = s[3..].parse().unwrap_or(256);
            if bits == 0 || bits > 256 || bits % 8 != 0 {
                return Err(format!("Invalid int size: {}", bits));
            }
            Ok(SolType::Int(bits))
        }
        _ if s.starts_with("bytes") => {
            let size: usize = s[5..]
                .parse()
                .map_err(|_| format!("Invalid bytes size: {}", s))?;
            if size == 0 || size > 32 {
                return Err(format!("Invalid bytes size: {}", size));
            }
            Ok(SolType::Bytes(size))
        }
        _ => Err(format!("Unknown ABI type: {}", s)),
    }
}

fn apply_array_suffix(base: SolType, suffix: &str) -> Result<SolType, String> {
    let suffix = suffix.trim();
    if suffix.is_empty() {
        return Ok(base);
    }
    if !suffix.starts_with('[') {
        return Err(format!("Expected '[' but found: {}", suffix));
    }
    let close = suffix
        .find(']')
        .ok_or_else(|| format!("Missing ']' in: {}", suffix))?;
    let size_str = &suffix[1..close];
    let rest = &suffix[close + 1..];

    let array_type = if size_str.is_empty() {
        SolType::Array(Box::new(base))
    } else {
        let size: usize = size_str
            .parse()
            .map_err(|_| format!("Invalid array size: {}", size_str))?;
        SolType::FixedArray(Box::new(base), size)
    };
    apply_array_suffix(array_type, rest)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if tuple output components have meaningful names (not all empty/positional).
fn has_named_components(outputs: &[AbiParam]) -> bool {
    if outputs.len() != 1 {
        return false;
    }
    let out = &outputs[0];
    if out.type_str != "tuple" && !out.type_str.starts_with("tuple[") {
        return false;
    }
    let comps = match &out.components {
        Some(c) if !c.is_empty() => c,
        _ => return false,
    };
    // Check that at least one component has a non-empty, non-positional name
    comps.iter().any(|c| !c.name.is_empty() && !c.name.starts_with("_field"))
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

fn generate_abi_reference_method(func: &AbiFunction) -> Result<TokenStream, String> {
    // Build the FunctionSignature for selector computation
    let input_types: Vec<SolType> = func
        .inputs
        .iter()
        .map(|p| parse_abi_type(&p.type_str, p.components.as_deref()))
        .collect::<Result<_, _>>()?;
    let output_types: Vec<SolType> = func
        .outputs
        .iter()
        .map(|p| parse_abi_type(&p.type_str, p.components.as_deref()))
        .collect::<Result<_, _>>()?;

    let sig = FunctionSignature {
        name: func.name.clone(),
        inputs: input_types.clone(),
        outputs: output_types.clone(),
    };

    let fn_name = format_ident!("{}", to_snake_case(&func.name));

    // --- Parameters ---
    let param_names: Vec<Ident> = func
        .inputs
        .iter()
        .map(|p| {
            let name = if p.name.is_empty() {
                format!("arg{}", 0)
            } else {
                to_snake_case(&p.name)
            };
            format_ident!("{}", name)
        })
        .collect();

    let params: Vec<TokenStream> = param_names
        .iter()
        .zip(input_types.iter())
        .map(|(name, ty): (&Ident, &SolType)| {
            let rt = ty.rust_type(true);
            quote! { #name: #rt }
        })
        .collect();

    // --- Selector + calldata init ---
    let [s0, s1, s2, s3] = compute_selector(&sig.canonical_signature());
    let selector_setup = quote! { let mut calldata = alloc::vec![#s0, #s1, #s2, #s3]; };

    // --- Encode statements ---
    let encodes: Vec<TokenStream> = param_names
        .iter()
        .zip(input_types.iter())
        .map(|(name, ty): (&Ident, &SolType)| {
            let enc = generate_encode(ty, quote!(#name), true);
            quote! { calldata.extend_from_slice(&#enc); }
        })
        .collect();

    // --- Return type and decode ---
    let (ret_ty, output_setup, decode_return, has_output) =
        generate_return_handling(func, &output_types)?;

    let output_arg = if has_output {
        quote! { Some(&mut output_ref) }
    } else {
        quote! { None }
    };

    Ok(quote! {
        pub fn #fn_name(&self, #(#params),*) -> pvm_contract::call::CallResult<#ret_ty> {
            extern crate alloc;
            #selector_setup
            #(#encodes)*
            #output_setup
            let mut output_ref: &mut [u8] = &mut output_buf[..];
            let result = <pvm_contract::api as pvm_contract::HostFn>::call_evm(
                pvm_contract::CallFlags::ALLOW_REENTRY,
                self.addr.as_fixed_bytes(), u64::MAX, &[0u8; 32], &calldata, #output_arg,
            );
            match result {
                Ok(()) => {
                    let written = output_ref.len();
                    let output = &output_buf[..written];
                    Ok(#decode_return)
                }
                Err(e) => Err(pvm_contract::call::CallError::from(e)),
            }
        }
    })
}

fn generate_return_handling(
    func: &AbiFunction,
    output_types: &[SolType],
) -> Result<(TokenStream, TokenStream, TokenStream, bool), String> {
    // Check if we should generate a named return struct
    if has_named_components(&func.outputs) {
        let comps = func.outputs[0].components.as_ref().unwrap();
        let struct_name = format_ident!("{}Return", to_pascal_case(&func.name));

        let comp_types: Vec<SolType> = comps
            .iter()
            .map(|c: &AbiParam| parse_abi_type(&c.type_str, c.components.as_deref()))
            .collect::<Result<_, _>>()?;

        let field_names: Vec<Ident> = comps
            .iter()
            .map(|c: &AbiParam| format_ident!("{}", to_snake_case(&c.name)))
            .collect();

        let output_size: usize = comp_types.iter().map(|t: &SolType| t.head_size()).sum();

        let mut offset = 0usize;
        let field_decodes: Vec<TokenStream> = comp_types
            .iter()
            .map(|t: &SolType| {
                let d = generate_decode(t, quote!(output), offset, true);
                offset += t.head_size();
                d
            })
            .collect();

        let field_assignments: Vec<TokenStream> = field_names
            .iter()
            .zip(field_decodes.iter())
            .map(|(name, decode): (&Ident, &TokenStream)| quote! { #name: #decode })
            .collect();

        let ret_ty: TokenStream = quote! { #struct_name };
        let output_setup: TokenStream = quote! { let mut output_buf = [0u8; #output_size]; };
        let decode_return: TokenStream = quote! {
            #struct_name {
                #(#field_assignments),*
            }
        };

        Ok((ret_ty, output_setup, decode_return, true))
    } else {
        match output_types {
            [] => {
                let ret_ty: TokenStream = quote! { () };
                let output_setup: TokenStream = quote! { let mut output_buf = [0u8; 0]; };
                let decode_return: TokenStream = quote! { () };
                Ok((ret_ty, output_setup, decode_return, false))
            }
            [one] => {
                let rt = one.rust_type(true);
                let output_size = one.head_size();
                let decode = generate_decode(one, quote!(output), 0, true);
                let ret_ty: TokenStream = quote! { #rt };
                let output_setup: TokenStream = quote! { let mut output_buf = [0u8; #output_size]; };
                Ok((ret_ty, output_setup, decode, true))
            }
            many => {
                let tys: Vec<TokenStream> = many.iter().map(|t: &SolType| t.rust_type(true)).collect();
                let output_size: usize = many.iter().map(|t: &SolType| t.head_size()).sum();
                let mut offset = 0usize;
                let decs: Vec<TokenStream> = many
                    .iter()
                    .map(|t: &SolType| {
                        let d = generate_decode(t, quote!(output), offset, true);
                        offset += t.head_size();
                        d
                    })
                    .collect();
                let ret_ty: TokenStream = quote! { (#(#tys),*) };
                let output_setup: TokenStream = quote! { let mut output_buf = [0u8; #output_size]; };
                let decode_return: TokenStream = quote! { (#(#decs),*) };
                Ok((ret_ty, output_setup, decode_return, true))
            }
        }
    }
}

/// Generate a named return struct for a function with tuple output that has named components.
fn generate_return_struct(func: &AbiFunction) -> Result<Option<TokenStream>, String> {
    if !has_named_components(&func.outputs) {
        return Ok(None);
    }
    let comps = func.outputs[0].components.as_ref().unwrap();
    let struct_name = format_ident!("{}Return", to_pascal_case(&func.name));

    let comp_types: Vec<SolType> = comps
        .iter()
        .map(|c| parse_abi_type(&c.type_str, c.components.as_deref()))
        .collect::<Result<_, _>>()?;

    let field_names: Vec<Ident> = comps
        .iter()
        .map(|c| format_ident!("{}", to_snake_case(&c.name)))
        .collect();

    let field_types: Vec<TokenStream> = comp_types.iter().map(|t: &SolType| t.rust_type(true)).collect();

    let result: TokenStream = quote! {
        pub struct #struct_name {
            #(pub #field_names: #field_types),*
        }
    };
    Ok(Some(result))
}

/// Generate the CDM reference function (mirrors `generate_cdm_reference` from contract.rs).
fn generate_cdm_reference(cdm_name: &str) -> TokenStream {
    let selector = compute_selector("getAddress(string)");
    let [s0, s1, s2, s3] = selector;

    quote! {
        /// The address of the contracts registry, baked in at compile time from
        /// the `CONTRACTS_REGISTRY_ADDR` environment variable.
        const __CDM_REGISTRY_ADDR: [u8; 20] = {
            const fn hex(c: u8) -> u8 {
                match c {
                    b'0'..=b'9' => c - b'0',
                    b'a'..=b'f' => c - b'a' + 10,
                    b'A'..=b'F' => c - b'A' + 10,
                    _ => panic!("Invalid hex character in CONTRACTS_REGISTRY_ADDR"),
                }
            }

            match option_env!("CONTRACTS_REGISTRY_ADDR") {
                Some(s) => {
                    let b = s.as_bytes();
                    let off = if b.len() > 1 && b[0] == b'0' && (b[1] == b'x' || b[1] == b'X') {
                        2
                    } else {
                        0
                    };
                    assert!(b.len() - off == 40, "CONTRACTS_REGISTRY_ADDR must be 40 hex chars (with optional 0x prefix)");
                    let mut r = [0u8; 20];
                    let mut i = 0;
                    while i < 20 {
                        r[i] = hex(b[off + i * 2]) << 4 | hex(b[off + i * 2 + 1]);
                        i += 1;
                    }
                    r
                }
                None => [0u8; 20],
            }
        };

        /// Get a runtime-resolved reference to this contract via CDM.
        ///
        /// Looks up the contract address from the ContractRegistry at runtime
        /// using the CDM name registered at compile time. The registry address
        /// is baked in from the `CONTRACTS_REGISTRY_ADDR` environment variable.
        ///
        /// # Panics
        ///
        /// Panics if the cross-contract call to the registry fails or the
        /// contract is not registered.
        pub fn cdm_reference() -> Reference {
            extern crate alloc;

            let cdm_name: &str = #cdm_name;
            let name_len = cdm_name.len();
            let padded_len = (name_len + 31) / 32 * 32;

            // Build calldata: selector + ABI-encoded string
            // String ABI encoding: offset (32 bytes) + length (32 bytes) + data (padded to 32)
            let mut calldata = alloc::vec![0u8; 4 + 32 + 32 + padded_len];

            // Selector for getAddress(string)
            calldata[0] = #s0;
            calldata[1] = #s1;
            calldata[2] = #s2;
            calldata[3] = #s3;

            // Offset to string data (always 32 = 0x20)
            calldata[4 + 24..4 + 32].copy_from_slice(&(32u64).to_be_bytes());

            // String length
            calldata[4 + 32 + 24..4 + 32 + 32].copy_from_slice(&(name_len as u64).to_be_bytes());

            // String data
            calldata[4 + 64..4 + 64 + name_len].copy_from_slice(cdm_name.as_bytes());

            // Output buffer: registry returns Option<Address> as tuple(bool isSome, address value)
            // ABI-encoded: 32 bytes for isSome + 32 bytes for address = 64 bytes
            let mut output_buf = [0u8; 64];
            let mut output_ref: &mut [u8] = &mut output_buf[..];

            let result = <pvm_contract::api as pvm_contract::HostFn>::call_evm(
                pvm_contract::CallFlags::ALLOW_REENTRY,
                &__CDM_REGISTRY_ADDR,
                u64::MAX,
                &[0u8; 32],
                &calldata,
                Some(&mut output_ref),
            );

            match result {
                Ok(()) => {
                    let written = output_ref.len();
                    let output = &output_buf[..written];
                    // First word (0..32) is isSome bool, second word (32..64) is the address
                    // Address is 20 bytes right-aligned in the second 32-byte word
                    let is_some = output[31] != 0;
                    if !is_some {
                        panic!("CDM: contract not found in registry");
                    }
                    let mut addr = [0u8; 20];
                    addr.copy_from_slice(&output[44..64]);
                    Reference::at(pvm_contract::Address::from(addr))
                }
                Err(_) => {
                    panic!("CDM: registry call failed");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Main expansion entry point
// ---------------------------------------------------------------------------

pub fn expand_abi_import(args: AbiImportArgs) -> syn::Result<TokenStream> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        syn::Error::new(proc_macro2::Span::call_site(), "CARGO_MANIFEST_DIR not set")
    })?;
    let full_path = std::path::Path::new(&manifest_dir).join(&args.abi_path);
    let json_str = std::fs::read_to_string(&full_path).map_err(|e| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("Failed to read {}: {}", full_path.display(), e),
        )
    })?;

    let functions = parse_abi_json(&json_str).map_err(|e| {
        syn::Error::new(proc_macro2::Span::call_site(), e)
    })?;

    // Generate return structs for functions with named tuple outputs
    let mut return_structs = Vec::new();
    for func in &functions {
        match generate_return_struct(func) {
            Ok(Some(s)) => return_structs.push(s),
            Ok(None) => {}
            Err(e) => {
                return Err(syn::Error::new(proc_macro2::Span::call_site(), e));
            }
        }
    }

    // Generate methods
    let mut methods = Vec::new();
    for func in &functions {
        let method = generate_abi_reference_method(func).map_err(|e| {
            syn::Error::new(proc_macro2::Span::call_site(), e)
        })?;
        methods.push(method);
    }

    let cdm_fn = if let Some(cdm_name) = &args.cdm {
        generate_cdm_reference(cdm_name)
    } else {
        quote! {}
    };

    let mod_name = format_ident!("{}", args.module_name);

    Ok(quote! {
        pub mod #mod_name {
            #(#return_structs)*

            #[derive(pvm_contract::Encode, pvm_contract::Decode)]
            pub struct Reference {
                addr: pvm_contract::Address,
            }

            impl Reference {
                pub fn at(addr: pvm_contract::Address) -> Self { Self { addr } }
                pub fn address(&self) -> &pvm_contract::Address { &self.addr }
                #(#methods)*
            }

            pub fn reference(addr: pvm_contract::Address) -> Reference {
                Reference::at(addr)
            }

            #cdm_fn
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_base_types() {
        assert_eq!(parse_abi_type("address", None).unwrap(), SolType::Address);
        assert_eq!(parse_abi_type("bool", None).unwrap(), SolType::Bool);
        assert_eq!(parse_abi_type("uint8", None).unwrap(), SolType::Uint(8));
        assert_eq!(parse_abi_type("uint256", None).unwrap(), SolType::Uint(256));
        assert_eq!(parse_abi_type("int64", None).unwrap(), SolType::Int(64));
        assert_eq!(parse_abi_type("bytes32", None).unwrap(), SolType::Bytes(32));
        assert_eq!(parse_abi_type("string", None).unwrap(), SolType::String);
        assert_eq!(parse_abi_type("bytes", None).unwrap(), SolType::DynBytes);
    }

    #[test]
    fn test_parse_array_types() {
        assert_eq!(
            parse_abi_type("uint256[]", None).unwrap(),
            SolType::Array(Box::new(SolType::Uint(256)))
        );
        assert_eq!(
            parse_abi_type("address[3]", None).unwrap(),
            SolType::FixedArray(Box::new(SolType::Address), 3)
        );
    }

    #[test]
    fn test_parse_tuple_type() {
        let comps = vec![
            AbiParam { name: "id".into(), type_str: "bytes32".into(), components: None },
            AbiParam { name: "status".into(), type_str: "uint8".into(), components: None },
            AbiParam { name: "proposer".into(), type_str: "address".into(), components: None },
        ];
        let result = parse_abi_type("tuple", Some(&comps)).unwrap();
        assert_eq!(
            result,
            SolType::Tuple(vec![SolType::Bytes(32), SolType::Uint(8), SolType::Address])
        );
    }

    #[test]
    fn test_parse_abi_json_basic() {
        let json = r#"[
            {"type":"constructor","inputs":[],"stateMutability":"nonpayable"},
            {"type":"function","name":"submitReview","inputs":[{"name":"subject","type":"address"},{"name":"job_id","type":"string"},{"name":"rating","type":"uint8"}],"outputs":[],"stateMutability":"nonpayable"},
            {"type":"function","name":"getAverageRating","inputs":[{"name":"subject","type":"address"}],"outputs":[{"name":"","type":"uint64"}],"stateMutability":"view"}
        ]"#;
        let funcs = parse_abi_json(json).unwrap();
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].name, "submitReview");
        assert_eq!(funcs[0].inputs.len(), 3);
        assert_eq!(funcs[1].name, "getAverageRating");
        assert_eq!(funcs[1].outputs.len(), 1);
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("getAverageRating"), "GetAverageRating");
        assert_eq!(to_pascal_case("submit_review"), "SubmitReview");
        assert_eq!(to_pascal_case("transfer"), "Transfer");
    }

    #[test]
    fn test_has_named_components() {
        // Single output with named tuple components
        let outputs = vec![AbiParam {
            name: "".into(),
            type_str: "tuple".into(),
            components: Some(vec![
                AbiParam { name: "id".into(), type_str: "bytes32".into(), components: None },
                AbiParam { name: "status".into(), type_str: "uint8".into(), components: None },
            ]),
        }];
        assert!(has_named_components(&outputs));

        // Single non-tuple output
        let outputs = vec![AbiParam {
            name: "".into(),
            type_str: "uint64".into(),
            components: None,
        }];
        assert!(!has_named_components(&outputs));

        // Multiple outputs (not a named struct case)
        let outputs = vec![
            AbiParam { name: "".into(), type_str: "uint64".into(), components: None },
            AbiParam { name: "".into(), type_str: "address".into(), components: None },
        ];
        assert!(!has_named_components(&outputs));
    }
}
