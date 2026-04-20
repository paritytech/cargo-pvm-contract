use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Ident, ItemMod, LitInt, LitStr, Token, parse::Parse, parse::ParseStream};

use super::abi_gen::generate_abi_gen;
use super::dispatch::{
    MethodInfo, RouteItems, generate_param_decoding, generate_revert_encoding, generate_router,
};
use crate::signature::{SolType, compute_selector};
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
    pub(super) constructor_inputs: Vec<(Ident, syn::Type)>,
    pub(super) fallback_name: Option<Ident>,
    pub(super) fallback_returns_result: bool,
    /// Error types from `Result<T, E>` return types, for ABI generation.
    pub(super) error_types: Vec<syn::Type>,
}

const VALID_PREFIXES: &[&str] = &["pvm", "pvm_contract", "pvm_contract_macros"];

/// Verify that a Rust method's parameters are compatible with the Solidity
/// function it implements. Checks arity strictly, and type compatibility for
/// non-custom types. Custom types (user-defined structs) are skipped because
/// they're resolved via trait `SOL_NAME` at runtime — we can't statically
/// verify them without expanding the full type graph.
fn check_signature_compatibility(
    func: &syn::ItemFn,
    sol_name: &str,
    sol_inputs: &[SolType],
    rust_param_types: &[syn::Type],
) -> syn::Result<()> {
    if sol_inputs.len() != rust_param_types.len() {
        return Err(syn::Error::new_spanned(
            func,
            format!(
                "Parameter count mismatch for `{sol_name}`: Solidity expects {}, Rust has {}",
                sol_inputs.len(),
                rust_param_types.len()
            ),
        ));
    }

    for (i, (sol_ty, rust_ty)) in sol_inputs.iter().zip(rust_param_types.iter()).enumerate() {
        let Some(rust_sol) = SolType::from_rust_type(rust_ty) else {
            continue; // unknown rust type, let downstream codegen produce a better error
        };
        // Skip type check if either side involves custom types — resolved via traits at runtime
        if sol_ty.has_custom_types() || rust_sol.has_custom_types() {
            continue;
        }
        if sol_ty != &rust_sol {
            return Err(syn::Error::new_spanned(
                rust_ty,
                format!(
                    "Parameter {} type mismatch for `{sol_name}`: Solidity `{}`, Rust maps to `{}`",
                    i,
                    sol_ty.canonical_name(),
                    rust_sol.canonical_name(),
                ),
            ));
        }
    }
    Ok(())
}

fn extract_method_rename(attrs: &[Attribute]) -> syn::Result<Option<String>> {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2
            && VALID_PREFIXES.contains(&segments[0].ident.to_string().as_str())
            && segments[1].ident == "method"
            && let syn::Meta::List(meta_list) = &attr.meta
            && let Ok(args) = syn::parse2::<super::method::MethodArgs>(meta_list.tokens.clone())
            && let Some(name) = args.rename
            && !name.is_empty()
        {
            if !is_valid_solidity_identifier(&name) {
                return Err(syn::Error::new_spanned(
                    attr,
                    format!(
                        "Invalid Solidity identifier `{name}`. \
                         Must match [a-zA-Z_$][a-zA-Z0-9_$]*"
                    ),
                ));
            }
            return Ok(Some(name));
        }
    }
    Ok(None)
}

fn is_valid_solidity_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

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

/// Collect the error type from a `Result<T, E>` return type, deduplicating by type name.
fn collect_error_type(
    output: &syn::ReturnType,
    error_types: &mut Vec<syn::Type>,
    seen: &mut Vec<String>,
) {
    if let Some(err_ty) = extract_error_type(output) {
        let name = quote::quote!(#err_ty).to_string();
        if !seen.contains(&name) {
            seen.push(name);
            error_types.push(err_ty);
        }
    }
}

/// Extract the error type `E` from a `Result<T, E>` return type.
fn extract_error_type(output: &syn::ReturnType) -> Option<syn::Type> {
    let syn::ReturnType::Type(_, ty) = output else {
        return None;
    };
    let syn::Type::Path(type_path) = ty.as_ref() else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    // Result<T, E> — E is the second generic argument
    let mut iter = args.args.iter();
    iter.next()?; // skip T
    let syn::GenericArgument::Type(error_ty) = iter.next()? else {
        return None;
    };
    Some(error_ty.clone())
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

fn extract_return_types(func: &syn::ItemFn) -> Vec<syn::Type> {
    match &func.sig.output {
        syn::ReturnType::Default => vec![],
        syn::ReturnType::Type(_, ty) => {
            if is_result_return_type(&func.sig.output) {
                extract_result_ok_type(ty).into_iter().collect()
            } else {
                extract_output_types(ty)
            }
        }
    }
}

fn extract_output_types(ty: &syn::Type) -> Vec<syn::Type> {
    if let syn::Type::Tuple(tuple) = ty {
        tuple.elems.iter().cloned().collect()
    } else {
        vec![ty.clone()]
    }
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
) -> syn::Result<Vec<(Ident, syn::Type)>> {
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
                Ok((ident, (*pat_type.ty).clone()))
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
    let mut fallback_returns_result = false;
    let mut implemented_sol_methods = Vec::new();
    let mut error_types: Vec<syn::Type> = Vec::new();
    let mut seen_error_names: Vec<String> = Vec::new();

    for item in &content.1 {
        if let syn::Item::Fn(func) = item {
            if has_pvm_attr(&func.attrs, "constructor") {
                has_constructor = true;
                constructor_name = Some(func.sig.ident.clone());
                constructor_returns_result = is_result_return_type(&func.sig.output);
                constructor_inputs = extract_typed_params(&func.sig.inputs)?;
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
            } else if has_pvm_attr(&func.attrs, "fallback") {
                has_fallback = true;
                fallback_name = Some(func.sig.ident.clone());
                fallback_returns_result = is_result_return_type(&func.sig.output);
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
            } else if has_pvm_attr(&func.attrs, "method") {
                let typed_params = extract_typed_params(&func.sig.inputs)?;
                let param_names: Vec<Ident> =
                    typed_params.iter().map(|(name, _)| name.clone()).collect();
                let param_types: Vec<syn::Type> =
                    typed_params.into_iter().map(|(_, ty)| ty).collect();

                let returns_result = is_result_return_type(&func.sig.output);
                let return_types = extract_return_types(func);

                let (sol_name, precomputed_selector) = if let Some(sol_iface) = sol_interface {
                    let rust_fn_name = func.sig.ident.to_string();
                    let rename = extract_method_rename(&func.attrs)?
                        .unwrap_or_else(|| to_snake_case(&rust_fn_name));
                    let sol_func = sol_iface
                        .functions
                        .iter()
                        .find(|f| f.name == rename || to_snake_case(&f.name) == rust_fn_name)
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                func,
                                format!(
                                    "No matching Solidity function found for `{rename}` in interface"
                                ),
                            )
                        })?;
                    check_signature_compatibility(
                        func,
                        &sol_func.name,
                        &sol_func.signature.inputs,
                        &param_types,
                    )?;
                    implemented_sol_methods.push(sol_func.name.clone());
                    let selector = compute_selector(&sol_func.signature.canonical_signature());
                    (sol_func.name.clone(), Some(selector))
                } else {
                    let sol_name = extract_method_rename(&func.attrs)?
                        .unwrap_or_else(|| to_camel_case(&func.sig.ident.to_string()));
                    (sol_name, None)
                };

                methods.push(MethodInfo {
                    fn_name: func.sig.ident.clone(),
                    sol_name,
                    param_names,
                    param_types,
                    return_types,
                    returns_result,
                    precomputed_selector,
                });
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
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
        fallback_returns_result,
        error_types,
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
    let (abi_gen_helper, abi_gen_main) = generate_abi_gen(&parsed, args.sol_path.is_some());

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

        let param_types: Vec<_> = parsed
            .constructor_inputs
            .iter()
            .map(|(_, ty)| ty.clone())
            .collect();
        let param_names: Vec<_> = parsed
            .constructor_inputs
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        let decoding = generate_param_decoding(&param_names, &param_types);
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
                let call_data_len = ::pvm_contract_types::PolkaVmHost::call_data_size() as usize;
                let mut call_data = alloc::vec![0u8; call_data_len];
                ::pvm_contract_types::PolkaVmHost::call_data_copy(&mut call_data, 0);
                let input = &call_data[..];
                #size_check
            }
        } else {
            let buffer_size = args.buffer_size;
            quote! {
                let call_data_len = ::pvm_contract_types::PolkaVmHost::call_data_size() as usize;
                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    ::pvm_contract_types::PolkaVmHost::return_value(
                        ::pvm_contract_types::ReturnFlags::REVERT,
                        &::pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE);
                }
                ::pvm_contract_types::PolkaVmHost::call_data_copy(&mut call_data[..call_data_len], 0);
                let input = &call_data[..call_data_len];
                #size_check
            }
        };

        let call_expr = quote! { #constructor_name(#(#call_args),*) };
        let revert_err = generate_revert_encoding(use_alloc);
        let decode_and_call = if parsed.constructor_returns_result {
            quote! {
                #(#decode_statements)*
                match #call_expr {
                    Ok(()) => {}
                    Err(e) => {
                        #revert_err
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

    let (route_items, router_impl) = generate_router(&parsed.methods, mod_name, use_alloc);
    let RouteItems {
        contract_struct,
        route_fn,
    } = route_items;
    let router_impl = router_impl.tokens;

    let (no_selector_handler, unknown_selector_handler) = if parsed.has_fallback {
        let fallback_name = parsed.fallback_name.as_ref().unwrap();
        let handler = if parsed.fallback_returns_result {
            let revert_err = generate_revert_encoding(use_alloc);
            quote! {
                match #fallback_name() {
                    Ok(()) => return,
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        } else {
            quote! {
                #fallback_name();
                return;
            }
        };
        (handler.clone(), handler)
    } else {
        (
            quote! {
                ::pvm_contract_types::PolkaVmHost::return_value(
                    ::pvm_contract_types::ReturnFlags::REVERT,
                    &::pvm_contract_types::framework_errors::NO_SELECTOR);
            },
            quote! {
                ::pvm_contract_types::PolkaVmHost::return_value(
                    ::pvm_contract_types::ReturnFlags::REVERT,
                    &::pvm_contract_types::framework_errors::UNKNOWN_SELECTOR);
            },
        )
    };

    let call_fn = if use_alloc {
        quote! {
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                let call_data_len = ::pvm_contract_types::PolkaVmHost::call_data_size() as usize;
                let mut call_data = alloc::vec![0u8; call_data_len];
                ::pvm_contract_types::PolkaVmHost::call_data_copy(&mut call_data, 0);

                if call_data_len < 4 {
                    #no_selector_handler
                }

                let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                let input = &call_data[4..];

                if route(selector, input).is_some() {
                    ::pvm_contract_types::PolkaVmHost::return_value(
                        ::pvm_contract_types::ReturnFlags::empty(), &[]);
                }

                #unknown_selector_handler
            }
        }
    } else {
        let buffer_size = args.buffer_size;
        quote! {
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                let call_data_len = ::pvm_contract_types::PolkaVmHost::call_data_size() as usize;
                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    ::pvm_contract_types::PolkaVmHost::return_value(
                        ::pvm_contract_types::ReturnFlags::REVERT,
                        &::pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE);
                }
                ::pvm_contract_types::PolkaVmHost::call_data_copy(&mut call_data[..call_data_len], 0);

                if call_data_len < 4 {
                    #no_selector_handler
                }

                let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                let input = &call_data[4..call_data_len];

                if route(selector, input).is_some() {
                    ::pvm_contract_types::PolkaVmHost::return_value(
                        ::pvm_contract_types::ReturnFlags::empty(), &[]);
                }

                #unknown_selector_handler
            }
        }
    };

    Ok(quote! {
        #alloc_setup

        #panic_handler

        #(#mod_attrs)*
        #mod_vis mod #mod_name {
            #mod_content

            #contract_struct

            #[cfg(not(feature = "abi-gen"))]
            #route_fn

            #[cfg(not(feature = "abi-gen"))]
            #call_fn

            #[cfg(not(feature = "abi-gen"))]
            #deploy_fn

            #abi_gen_helper
        }

        #[cfg(not(feature = "abi-gen"))]
        #router_impl

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
                // Gate user functions behind not(abi-gen): abi-gen only needs
                // type info (`SolEncode::SOL_NAME`) — function bodies may call
                // host APIs that don't exist on the native target used for
                // abi-gen compilation.
                quote! {
                    #[cfg(not(feature = "abi-gen"))]
                    #new_func
                }
            }
            syn::Item::Use(use_item) => {
                // Gate `use alloc::*` imports behind not(abi-gen): when the
                // allocator's `extern crate alloc` is cfg-gated out, these
                // would fail to resolve on the host target.
                let use_str = quote! { #use_item }.to_string();
                if use_str.contains("alloc ::") || use_str.contains("alloc::") {
                    quote! {
                        #[cfg(not(feature = "abi-gen"))]
                        #use_item
                    }
                } else {
                    quote! { #use_item }
                }
            }
            other => quote! { #other },
        })
        .collect();

    quote! {
        #[cfg(not(feature = "abi-gen"))]
        #[allow(unused_imports)]
        use ::pvm_contract_types::HostApi as _;

        #(#items)*
    }
}

#[cfg(test)]
mod tests {
    use super::{ContractArgs, expand_contract};
    use syn::ItemMod;

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
    fn constructor_with_params_generates_deploy_decoding() {
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

        // deploy() is generated with param decoding
        assert!(output.contains("deploy"));
        // param names are used in decode statements
        assert!(output.contains("\"owner\""));
        assert!(output.contains("\"supply\""));
        // route function is generated
        assert!(output.contains("fn route"));
    }

    #[test]
    fn generates_router_impl_and_route_fn() {
        let item: syn::ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[pvm_contract_macros::constructor]
                pub fn new() {}

                #[pvm_contract_macros::method]
                pub fn balance_of(account: Address) -> U256 {
                    U256::ZERO
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        // Router struct is generated inside the module
        assert!(output.contains("pub struct Contract"));
        // route() function is generated
        assert!(
            output.contains("fn route (selector : [u8 ; 4] , input : & [u8]) -> Option < () >")
        );
        // Router trait impl references the module
        assert!(
            output.contains("impl :: pvm_contract_types :: Router for my_contract :: Contract")
        );
        // call() delegates to route()
        assert!(output.contains("route (selector , input)"));
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

    #[test]
    fn user_functions_are_cfg_gated_for_abi_gen() {
        let item: syn::ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[pvm_contract_macros::constructor]
                pub fn new() {}

                #[pvm_contract_macros::method]
                pub fn do_something(value: U256) -> U256 {
                    value
                }
            }
        "#,
        )
        .unwrap();

        let tokens = expand_contract(ContractArgs::default(), item).unwrap();
        let output = tokens.to_string();
        let pretty = prettyplease::unparse(
            &syn::parse_file(&output).expect("expanded output should be valid Rust"),
        );

        // User functions should be gated behind not(abi-gen) so they don't
        // compile on the host target (they may call host APIs).
        assert!(
            output.contains("not (feature = \"abi-gen\")"),
            "user functions must be cfg-gated for abi-gen.\nExpanded output:\n{pretty}"
        );

        // The abi-gen helper should still reference the type for SOL_NAME
        assert!(
            output.contains("SOL_NAME"),
            "abi-gen helper must reference SOL_NAME.\nExpanded output:\n{pretty}"
        );
    }

    #[test]
    fn error_paths_do_not_emit_raw_bytes() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[pvm_contract_macros::constructor]
                pub fn new() -> Result<(), MyError> {
                    Ok(())
                }

                #[pvm_contract_macros::method]
                pub fn transfer(to: u64) -> Result<(), MyError> {
                    Ok(())
                }

                #[pvm_contract_macros::fallback]
                pub fn fallback() -> Result<(), MyError> {
                    Ok(())
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        // Error paths must not use raw bytes (e.as_ref()) — regression guard
        assert!(
            !output.contains("as_ref"),
            "Generated dispatch should not use as_ref for error encoding"
        );
    }

    #[test]
    fn fallback_with_unit_return_generates_plain_call() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[pvm_contract_macros::constructor]
                pub fn new() {}

                #[pvm_contract_macros::fallback]
                pub fn fallback() {}
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        // Unit-return fallback should generate a plain call, not a match on Ok/Err
        assert!(
            output.contains("fallback ()"),
            "Unit-return fallback should generate a direct call"
        );
        assert!(
            !output.contains("match fallback"),
            "Unit-return fallback should not generate a match expression"
        );
    }
}
