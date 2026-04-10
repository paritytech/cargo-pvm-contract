use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Ident, ItemMod, LitInt, LitStr, Token, parse::Parse, parse::ParseStream};

use super::abi_gen::generate_abi_gen_main;
use super::dispatch::{MethodInfo, generate_dispatch_arm, generate_param_decoding};
use crate::signature::{FunctionSignature, SolType};
use crate::solidity::{SolInterface, parse_solidity_interface, to_snake_case};

#[derive(Debug, PartialEq, Eq)]
pub struct ContractArgs {
    pub buffer_size: usize,
    pub sol_path: Option<String>,
    pub allocator: Option<AllocatorKind>,
    pub allocator_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatorKind {
    Pico,
    Bump,
}

impl Default for ContractArgs {
    fn default() -> Self {
        ContractArgs {
            buffer_size: 256,
            sol_path: None,
            allocator: None,
            allocator_size: 1024,
        }
    }
}

impl Parse for ContractArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ContractArgs::default();
        let mut allocator_size_set = false;

        if input.peek(LitStr) {
            let path: LitStr = input.parse()?;
            args.sol_path = Some(path.value());
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            match ident.to_string().as_str() {
                "no_alloc" => {
                    return Err(syn::Error::new(
                        ident.span(),
                        "`no_alloc` was removed. no-alloc is now the default. Use `buffer = N` to customize stack calldata buffer size.",
                    ));
                }
                "buffer" => {
                    input.parse::<Token![=]>()?;
                    let size: LitInt = input.parse()?;
                    args.buffer_size = size.base10_parse()?;
                }
                "allocator" => {
                    input.parse::<Token![=]>()?;
                    let allocator: LitStr = input.parse()?;
                    args.allocator = Some(match allocator.value().as_str() {
                        "pico" | "picoalloc" | "picoallocator" => AllocatorKind::Pico,
                        "bump" => AllocatorKind::Bump,
                        other => {
                            return Err(syn::Error::new(
                                allocator.span(),
                                format!("Unknown allocator `{other}`. Expected `pico` or `bump`."),
                            ));
                        }
                    });
                }
                "allocator_size" => {
                    input.parse::<Token![=]>()?;
                    let size: LitInt = input.parse()?;
                    args.allocator_size = size.base10_parse()?;
                    allocator_size_set = true;
                }
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("Unknown argument: {other}"),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        if allocator_size_set
            && !matches!(
                args.allocator,
                Some(AllocatorKind::Pico) | Some(AllocatorKind::Bump)
            )
        {
            return Err(syn::Error::new(
                input.span(),
                "`allocator_size` requires `allocator = \"pico\"` or `allocator = \"bump\"`",
            ));
        }

        Ok(args)
    }
}

fn load_sol_interface(path: &str) -> Result<SolInterface, String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "CARGO_MANIFEST_DIR not set".to_string())?;
    let full_path = std::path::Path::new(&manifest_dir).join(path);
    let source = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("Failed to read {}: {}", full_path.display(), e))?;
    parse_solidity_interface(&source)
}

pub(super) struct ParsedContract {
    pub(super) mod_name: Ident,
    pub(super) methods: Vec<MethodInfo>,
    pub(super) has_constructor: bool,
    pub(super) has_fallback: bool,
    pub(super) constructor_name: Option<Ident>,
    pub(super) constructor_returns_result: bool,
    pub(super) constructor_inputs: Vec<(Ident, SolType)>,
    pub(super) fallback_name: Option<Ident>,
}

fn extract_method_rename(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2
            && (segments[0].ident == "pvm" || segments[0].ident == "pvm_contract")
            && segments[1].ident == "method"
            && let syn::Meta::List(meta_list) = &attr.meta
        {
            let tokens_str = meta_list.tokens.to_string();
            if let Some(start) = tokens_str.find("rename") {
                let after_rename = &tokens_str[start..];
                if let Some(eq_pos) = after_rename.find('=') {
                    let after_eq = after_rename[eq_pos + 1..].trim();
                    let name = after_eq.trim_matches(|c| c == '"' || c == ' ');
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}

const VALID_PREFIXES: &[&str] = &["pvm", "pvm_contract", "pvm_contract_macros"];

fn has_pvm_attr(attrs: &[Attribute], name: &str) -> bool {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2 {
            let first = segments[0].ident.to_string();
            if VALID_PREFIXES.contains(&first.as_str()) && segments[1].ident == name {
                return true;
            }
        }
    }
    false
}

fn is_result_return_type(output: &syn::ReturnType) -> bool {
    match output {
        syn::ReturnType::Default => false,
        syn::ReturnType::Type(_, ty) => {
            if let syn::Type::Path(type_path) = ty.as_ref()
                && let Some(segment) = type_path.path.segments.last()
            {
                return segment.ident == "Result";
            }
            false
        }
    }
}

fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for (i, c) in s.chars().enumerate() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else if i == 0 {
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }

    result
}

fn infer_signature_from_rust(func: &syn::ItemFn) -> syn::Result<FunctionSignature> {
    let rust_name = func.sig.ident.to_string();
    let sol_name = to_camel_case(&rust_name);

    let inputs: Vec<SolType> = func
        .sig
        .inputs
        .iter()
        .filter_map(|arg| {
            if let syn::FnArg::Typed(pat_type) = arg {
                SolType::from_rust_type(&pat_type.ty)
            } else {
                None
            }
        })
        .collect();

    let outputs = match &func.sig.output {
        syn::ReturnType::Default => vec![],
        syn::ReturnType::Type(_, ty) => {
            if is_result_return_type(&func.sig.output) {
                extract_result_ok_type(ty)
                    .and_then(|inner| SolType::from_rust_type(&inner))
                    .into_iter()
                    .collect()
            } else {
                SolType::from_rust_type(ty).into_iter().collect()
            }
        }
    };

    Ok(FunctionSignature {
        name: sol_name,
        inputs,
        outputs,
    })
}

fn extract_result_ok_type(ty: &syn::Type) -> Option<syn::Type> {
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first()
    {
        if let syn::Type::Tuple(tuple) = ok_ty
            && tuple.elems.is_empty()
        {
            return None;
        }
        return Some(ok_ty.clone());
    }
    None
}

fn extract_typed_params(
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
) -> syn::Result<Vec<(Ident, SolType)>> {
    inputs
        .iter()
        .map(|arg| {
            if let syn::FnArg::Typed(pat_type) = arg {
                let ident = if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    pat_ident.ident.clone()
                } else {
                    return Err(syn::Error::new_spanned(
                        &pat_type.pat,
                        "Parameters must be simple identifiers",
                    ));
                };
                let sol_type = SolType::from_rust_type(&pat_type.ty).ok_or_else(|| {
                    syn::Error::new_spanned(
                        &pat_type.ty,
                        "Cannot map parameter type to a Solidity type",
                    )
                })?;
                Ok((ident, sol_type))
            } else {
                Err(syn::Error::new_spanned(arg, "Unexpected `self` parameter"))
            }
        })
        .collect()
}

fn parse_contract(
    input: &ItemMod,
    sol_interface: Option<&SolInterface>,
) -> syn::Result<ParsedContract> {
    let mod_name = input.ident.clone();
    let content = input
        .content
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(input, "Contract module must have a body"))?;

    let mut methods = Vec::new();
    let mut has_constructor = false;
    let mut has_fallback = false;
    let mut constructor_name = None;
    let mut constructor_returns_result = false;
    let mut constructor_inputs = Vec::new();
    let mut fallback_name = None;
    let mut implemented_sol_methods = Vec::new();

    for item in &content.1 {
        if let syn::Item::Fn(func) = item {
            if has_pvm_attr(&func.attrs, "constructor") {
                has_constructor = true;
                constructor_name = Some(func.sig.ident.clone());
                constructor_returns_result = is_result_return_type(&func.sig.output);
                constructor_inputs = extract_typed_params(&func.sig.inputs)?;
            } else if has_pvm_attr(&func.attrs, "fallback") {
                has_fallback = true;
                fallback_name = Some(func.sig.ident.clone());
            } else if has_pvm_attr(&func.attrs, "method") {
                let param_names: Vec<Ident> = extract_typed_params(&func.sig.inputs)?
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect();

                let returns_result = is_result_return_type(&func.sig.output);

                let sol_fn_name = extract_method_rename(&func.attrs)
                    .unwrap_or_else(|| to_snake_case(&func.sig.ident.to_string()));

                let signature = if let Some(sol_iface) = sol_interface {
                    let rust_fn_name = func.sig.ident.to_string();
                    let sol_func = sol_iface
                        .functions
                        .iter()
                        .find(|f| f.name == sol_fn_name || to_snake_case(&f.name) == rust_fn_name)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                func,
                                format!(
                                    "No matching Solidity function found for `{sol_fn_name}` in interface"
                                ),
                            )
                        })?;
                    implemented_sol_methods.push(sol_func.name.clone());
                    sol_func.signature.clone()
                } else {
                    let mut sig = infer_signature_from_rust(func)?;
                    if let Some(rename) = extract_method_rename(&func.attrs) {
                        sig.name = rename;
                    }
                    sig
                };

                methods.push(MethodInfo {
                    fn_name: func.sig.ident.clone(),
                    signature,
                    param_names,
                    returns_result,
                });
            }
        }
    }

    if let Some(sol_iface) = sol_interface {
        let missing: Vec<_> = sol_iface
            .functions
            .iter()
            .filter(|f| !implemented_sol_methods.contains(&f.name))
            .map(|f| f.name.as_str())
            .collect();

        if !missing.is_empty() {
            return Err(syn::Error::new_spanned(
                input,
                format!(
                    "Missing implementations for Solidity functions: {}",
                    missing.join(", ")
                ),
            ));
        }
    }

    Ok(ParsedContract {
        mod_name,
        methods,
        has_constructor,
        has_fallback,
        constructor_name,
        constructor_returns_result,
        constructor_inputs,
        fallback_name,
    })
}

pub fn expand_contract(args: ContractArgs, input: ItemMod) -> syn::Result<TokenStream> {
    let sol_interface = if let Some(ref path) = args.sol_path {
        Some(load_sol_interface(path).map_err(|e| syn::Error::new_spanned(&input, e))?)
    } else {
        None
    };

    let parsed = parse_contract(&input, sol_interface.as_ref())?;
    let use_alloc = args.allocator.is_some();
    let abi_gen_main = generate_abi_gen_main(&parsed, args.sol_path.is_some());

    let mod_name = &parsed.mod_name;
    let mod_vis = &input.vis;
    let mod_attrs = &input.attrs;

    let mod_content = strip_pvm_attrs(&input);

    let alloc_setup = match args.allocator {
        Some(AllocatorKind::Pico) => {
            let allocator_size = args.allocator_size;
            quote! {
                #[cfg(not(feature = "abi-gen"))]
                extern crate alloc;

                #[cfg(not(feature = "abi-gen"))]
                use alloc::vec;

                #[cfg(not(feature = "abi-gen"))]
                use alloc::vec::Vec;

                #[cfg(not(feature = "abi-gen"))]
                #[global_allocator]
                static mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<#allocator_size>>> = {
                    static mut ARRAY: picoalloc::Array<#allocator_size> = picoalloc::Array([0u8; #allocator_size]);

                    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
                        picoalloc::ArrayPointer::new(&raw mut ARRAY)
                    }))
                };
            }
        }
        Some(AllocatorKind::Bump) => {
            let allocator_size = args.allocator_size;
            quote! {
                #[cfg(not(feature = "abi-gen"))]
                extern crate alloc;

                #[cfg(not(feature = "abi-gen"))]
                use alloc::vec;

                #[cfg(not(feature = "abi-gen"))]
                use alloc::vec::Vec;

                #[cfg(not(feature = "abi-gen"))]
                #[global_allocator]
                static ALLOC: pvm_bump_allocator::BumpAllocator<#allocator_size> =
                    pvm_bump_allocator::BumpAllocator::new();
            }
        }
        None => quote! {},
    };

    let panic_handler = quote! {
        #[cfg(all(
            not(feature = "abi-gen"),
            any(target_arch = "riscv32", target_arch = "riscv64")
        ))]
        #[panic_handler]
        fn panic(_info: &core::panic::PanicInfo) -> ! {
            unsafe {
                core::arch::asm!("unimp");
                core::hint::unreachable_unchecked()
            }
        }
    };

    let deploy_fn = if parsed.has_constructor {
        let constructor_name = parsed.constructor_name.as_ref().unwrap();

        let sol_types: Vec<_> = parsed
            .constructor_inputs
            .iter()
            .map(|(_, ty)| ty.clone())
            .collect();
        let param_names: Vec<_> = parsed
            .constructor_inputs
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        let decoding = generate_param_decoding(&param_names, &sol_types, use_alloc);
        let super::dispatch::ParamDecoding {
            size_check,
            decode_statements,
            call_args,
        } = decoding;

        // Constructor calldata has no 4-byte selector prefix (unlike `call()`),
        // so the entire calldata is ABI-encoded args.
        let read_calldata = if param_names.is_empty() {
            quote! {}
        } else if use_alloc {
            quote! {
                let call_data_len = pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = vec![0u8; call_data_len];
                pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data, 0);
                let input = &call_data[..];
                #size_check
            }
        } else {
            let buffer_size = args.buffer_size;
            quote! {
                let call_data_len = pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    pallet_revive_uapi::HostFnImpl::return_value(
                        pallet_revive_uapi::ReturnFlags::REVERT, b"CalldataTooLarge");
                }
                pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data[..call_data_len], 0);
                let input = &call_data[..call_data_len];
                #size_check
            }
        };

        let call_expr = quote! { #mod_name::#constructor_name(#(#call_args),*) };
        let decode_and_call = if parsed.constructor_returns_result {
            quote! {
                #(#decode_statements)*
                match #call_expr {
                    Ok(()) => {}
                    Err(e) => {
                        pallet_revive_uapi::HostFnImpl::return_value(
                            pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
                    }
                }
            }
        } else {
            quote! {
                #(#decode_statements)*
                #call_expr;
            }
        };

        quote! {
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn deploy() {
                #read_calldata
                #decode_and_call
            }
        }
    } else {
        quote! {
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn deploy() {}
        }
    };

    let (selector_consts, dispatch_arms): (Vec<_>, Vec<_>) = parsed
        .methods
        .iter()
        .map(|m| generate_dispatch_arm(m, mod_name, use_alloc))
        .unzip();

    let fallback_handler = if parsed.has_fallback {
        let fallback_name = parsed.fallback_name.as_ref().unwrap();
        quote! {
            match #mod_name::#fallback_name() {
                Ok(()) => return,
                Err(e) => {
                    pallet_revive_uapi::HostFnImpl::return_value(
                        pallet_revive_uapi::ReturnFlags::REVERT, e.as_ref());
                }
            }
        }
    } else {
        quote! {
            pallet_revive_uapi::HostFnImpl::return_value(
                pallet_revive_uapi::ReturnFlags::REVERT, b"");
        }
    };

    let call_fn = if use_alloc {
        quote! {
            #[allow(non_upper_case_globals)]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                let call_data_len = pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = vec![0u8; call_data_len];
                pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data, 0);

                if call_data_len < 4 {
                    #fallback_handler
                }

                let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                let input = &call_data[4..];

                #(#selector_consts)*

                match selector {
                    #(#dispatch_arms)*
                    _ => {
                        #fallback_handler
                    }
                }
            }
        }
    } else {
        let buffer_size = args.buffer_size;
        quote! {
            #[allow(non_upper_case_globals)]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                let call_data_len = pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    pallet_revive_uapi::HostFnImpl::return_value(
                        pallet_revive_uapi::ReturnFlags::REVERT, b"CalldataTooLarge");
                }
                pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data[..call_data_len], 0);

                if call_data_len < 4 {
                    #fallback_handler
                }

                let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                let input = &call_data[4..call_data_len];

                #(#selector_consts)*

                match selector {
                    #(#dispatch_arms)*
                    _ => {
                        #fallback_handler
                    }
                }
            }
        }
    };

    Ok(quote! {
        #[cfg(not(feature = "abi-gen"))]
        use pallet_revive_uapi::HostFn as _;

        #alloc_setup

        #panic_handler

        #[cfg(not(feature = "abi-gen"))]
        #deploy_fn

        #[cfg(not(feature = "abi-gen"))]
        #call_fn

        #(#mod_attrs)*
        #mod_vis mod #mod_name {
            #mod_content
        }

        #abi_gen_main
    })
}

fn strip_pvm_attrs(input: &ItemMod) -> TokenStream {
    let content = input.content.as_ref().unwrap();
    let items: Vec<_> = content
        .1
        .iter()
        .map(|item| match item {
            syn::Item::Fn(func) => {
                let mut new_func = func.clone();
                new_func.attrs.retain(|attr| {
                    let segments: Vec<_> = attr.path().segments.iter().collect();
                    !(segments.len() == 2
                        && VALID_PREFIXES.contains(&segments[0].ident.to_string().as_str())
                        && (segments[1].ident == "method"
                            || segments[1].ident == "constructor"
                            || segments[1].ident == "fallback"))
                });
                quote! { #new_func }
            }
            other => quote! { #other },
        })
        .collect();

    quote! {
        #[allow(unused_imports)]
        use pallet_revive_uapi::HostFn as _;

        #(#items)*
    }
}

#[cfg(test)]
mod tests {
    use super::{ContractArgs, expand_contract};

    #[test]
    fn parses_no_alloc_with_nested_buffer() {
        let args = syn::parse_str::<ContractArgs>("\"MyToken.sol\", buffer = 512")
            .expect("top-level buffer should parse");

        assert_eq!(
            args,
            ContractArgs {
                buffer_size: 512,
                sol_path: Some("MyToken.sol".to_string()),
                allocator: None,
                allocator_size: 1024,
            }
        );
    }

    #[test]
    fn parses_pico_allocator_with_custom_size() {
        let args = syn::parse_str::<ContractArgs>("allocator = \"pico\", allocator_size = 2048")
            .expect("pico allocator with custom size should parse");

        assert_eq!(
            args,
            ContractArgs {
                buffer_size: 256,
                sol_path: None,
                allocator: Some(super::AllocatorKind::Pico),
                allocator_size: 2048,
            }
        );
    }

    #[test]
    fn rejects_removed_no_alloc_argument() {
        let error = syn::parse_str::<ContractArgs>("no_alloc")
            .expect_err("removed no_alloc argument should be rejected");

        assert!(error.to_string().contains("`no_alloc` was removed"));
    }

    #[test]
    fn accepts_allocator_size_with_bump() {
        let args = syn::parse_str::<ContractArgs>("allocator = \"bump\", allocator_size = 2048")
            .expect("allocator_size should be accepted with bump allocator");
        assert_eq!(args.allocator_size, 2048);
    }

    #[test]
    fn rejects_allocator_size_without_allocator() {
        let error = syn::parse_str::<ContractArgs>("allocator_size = 2048")
            .expect_err("allocator_size should require an allocator");

        assert!(error.to_string().contains("`allocator_size` requires"));
    }

    #[test]
    fn constructor_inputs_appear_in_abi_gen_output() {
        let item: syn::ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[pvm_contract_macros::constructor]
                pub fn new(owner: Address, supply: U256) {}
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        // abi-gen main is generated (no sol_path)
        assert!(output.contains("feature = \"abi-gen\""));
        // constructor entry is present with its inputs array
        assert!(output.contains("{\\\"type\\\":\\\"constructor\\\",\\\"inputs\\\":["));
        // param names are emitted
        assert!(output.contains("\"owner\""));
        assert!(output.contains("\"supply\""));
        // param types are emitted
        assert!(output.contains("\"address\""));
        assert!(output.contains("\"uint256\""));
    }

    #[test]
    fn constructor_with_result_and_inputs_generates_match_and_decode() {
        let item: syn::ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[pvm_contract_macros::constructor]
                pub fn new(owner: Address) -> Result<(), Error> {
                    Ok(())
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        // Should have decode logic for the input
        assert!(output.contains("\"owner\""));
        // Should have match for Result error handling
        assert!(output.contains("Err (e)"));
        assert!(output.contains("REVERT"));
    }
}
