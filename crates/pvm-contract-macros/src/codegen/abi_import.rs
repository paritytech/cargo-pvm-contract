use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Ident, LitStr, Token};

use crate::codegen::decode::generate_decode;
use crate::codegen::encode::generate_encode_params;
use crate::codegen::generate_cdm_reference;
use crate::codegen::generate_resolved_return_parts;
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
    /// `true` for the `cdm::import!`-emitted form
    /// (`#![abi_import(alloc = true)] <ident>, "<path>"`), which expects a
    /// 4-generic typestate-shaped contract type rather than the legacy
    /// `Reference` struct. See [`expand_abi_import`].
    pub typestate: bool,
}

impl Parse for AbiImportArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // New `cdm::import!` form, emitted as:
        //   abi_import! { #![abi_import(alloc = true)] <ident>, "<abs path>" }
        // Discriminated by a leading inner attribute, or a leading ident
        // (the legacy form always starts with a string literal). We ignore the
        // `alloc` flag — this branch is always alloc-enabled for imports.
        let inner_attrs = Attribute::parse_inner(input)?;
        if !inner_attrs.is_empty() || input.peek(Ident) {
            let name: Ident = input.parse()?;
            input.parse::<Token![,]>()?;
            let abi_path: LitStr = input.parse()?;
            let _ = input.parse::<Option<Token![,]>>()?;
            return Ok(AbiImportArgs {
                module_name: name.to_string(),
                abi_path: abi_path.value(),
                cdm: None,
                typestate: true,
            });
        }

        // Legacy form: "name", "path" [, cdm = "@org/x"]
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
            typestate: false,
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
    let encode_block = generate_encode_params(&param_names, &input_types);

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
            #encode_block
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
        Ok(generate_resolved_return_parts(output_types))
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

    let mod_name = format_ident!("{}", args.module_name);

    // New `cdm::import!` form: emit a 4-generic, typestate-shaped contract type
    // (`Foo<M, I, O, const INITIALIZED: bool>`) with a `from_address`
    // constructor on `<Pure, (), (), false>`, so `pvm_cdm::reference!` can
    // attach `cdm_lookup()` / `cdm_from_env()`. The imported call methods are
    // the SAME direct-call methods used by the legacy `Reference` — they read
    // `self.addr` and route through `api::call_evm`, so no CallBuilder /
    // ContractContext layer is needed.
    if args.typestate {
        let type_name = format_ident!("{}", to_pascal_case(&args.module_name));
        return Ok(quote! {
            pub mod #mod_name {
                #(#return_structs)*

                pub struct #type_name<
                    M = pvm_contract::Pure,
                    I = (),
                    O = (),
                    const INITIALIZED: bool = false,
                > {
                    addr: pvm_contract::Address,
                    _cdm_marker: core::marker::PhantomData<(M, I, O)>,
                }

                impl #type_name<pvm_contract::Pure, (), (), false> {
                    /// Construct a handle to an already-deployed instance at `addr`.
                    pub fn from_address(addr: pvm_contract::Address) -> Self {
                        Self { addr, _cdm_marker: core::marker::PhantomData }
                    }

                    /// The on-chain address this handle targets.
                    pub fn address(&self) -> pvm_contract::Address {
                        self.addr
                    }

                    #(#methods)*
                }
            }
        });
    }

    let cdm_fn = if let Some(cdm_name) = &args.cdm {
        generate_cdm_reference(cdm_name)
    } else {
        quote! {}
    };

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
