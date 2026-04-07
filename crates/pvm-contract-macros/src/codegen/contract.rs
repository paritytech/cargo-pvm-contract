use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse::ParseStream, Attribute, Ident, ItemMod, LitInt, LitStr, Token};

use super::decode::{calculate_min_input_size, generate_decode_params};
use super::dispatch::{generate_dispatch_arm, generate_trait_dispatch_arm, MethodInfo};
use super::encode::{generate_encode_params, generate_encode_params_trait};
use super::{generate_cdm_reference, generate_resolved_return_parts, unwrap_option_inner};
use crate::signature::{compute_selector, FunctionSignature, SolType};
use crate::solidity::{parse_solidity_interface, to_snake_case, SolInterface};

pub struct ContractArgs {
    pub no_alloc: bool,
    pub buffer_size: usize,
    pub sol_path: Option<String>,
    pub cdm: Option<String>,
}

impl Default for ContractArgs {
    fn default() -> Self {
        ContractArgs {
            no_alloc: false,
            buffer_size: 256,
            sol_path: None,
            cdm: None,
        }
    }
}

impl Parse for ContractArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ContractArgs::default();

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
                    args.no_alloc = true;
                }
                "buffer" => {
                    input.parse::<Token![=]>()?;
                    let size: LitInt = input.parse()?;
                    args.buffer_size = size.base10_parse()?;
                }
                "cdm" => {
                    input.parse::<Token![=]>()?;
                    let name: LitStr = input.parse()?;
                    args.cdm = Some(name.value());
                }
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("Unknown argument: {}", other),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
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

struct ConstructorInfo {
    fn_name: Ident,
    param_names: Vec<Ident>,
    param_types: Vec<SolType>,
    raw_param_types: Vec<syn::Type>,
    all_resolved: bool,
}

struct ParsedContract {
    mod_name: Ident,
    methods: Vec<MethodInfo>,
    has_fallback: bool,
    constructor: Option<ConstructorInfo>,
    fallback_name: Option<Ident>,
}

fn extract_method_rename(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2
            && (segments[0].ident == "pvm" || segments[0].ident == "pvm_contract")
            && segments[1].ident == "method"
        {
            if let syn::Meta::List(meta_list) = &attr.meta {
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
    }
    None
}

fn has_pvm_attr(attrs: &[Attribute], name: &str) -> bool {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2 && segments[0].ident == "pvm" && segments[1].ident == name {
            return true;
        }
    }
    false
}

fn is_result_return_type(output: &syn::ReturnType) -> bool {
    match output {
        syn::ReturnType::Default => false,
        syn::ReturnType::Type(_, ty) => {
            if let syn::Type::Path(type_path) = ty.as_ref() {
                if let Some(segment) = type_path.path.segments.last() {
                    return segment.ident == "Result";
                }
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
    if let syn::Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            if segment.ident == "Result" {
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    if let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first() {
                        if let syn::Type::Tuple(tuple) = ok_ty {
                            if tuple.elems.is_empty() {
                                return None;
                            }
                        }
                        return Some(ok_ty.clone());
                    }
                }
            }
        }
    }
    None
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
    let mut has_fallback = false;
    let mut constructor: Option<ConstructorInfo> = None;
    let mut fallback_name = None;
    let mut implemented_sol_methods = Vec::new();

    for item in &content.1 {
        if let syn::Item::Fn(func) = item {
            if has_pvm_attr(&func.attrs, "constructor") {
                let param_names: Vec<Ident> = func.sig.inputs.iter().filter_map(|arg| {
                    if let syn::FnArg::Typed(pat_type) = arg {
                        if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                            return Some(pat_ident.ident.clone());
                        }
                    }
                    None
                }).collect();
                let raw_param_types: Vec<syn::Type> = func.sig.inputs.iter().filter_map(|arg| {
                    if let syn::FnArg::Typed(pat_type) = arg {
                        Some((*pat_type.ty).clone())
                    } else {
                        None
                    }
                }).collect();
                let param_types: Vec<SolType> = func.sig.inputs.iter().filter_map(|arg| {
                    if let syn::FnArg::Typed(pat_type) = arg {
                        SolType::from_rust_type(&pat_type.ty)
                    } else {
                        None
                    }
                }).collect();
                let all_resolved = raw_param_types.iter().all(|ty| SolType::from_rust_type(ty).is_some());
                constructor = Some(ConstructorInfo {
                    fn_name: func.sig.ident.clone(),
                    param_names,
                    param_types,
                    raw_param_types,
                    all_resolved,
                });
            } else if has_pvm_attr(&func.attrs, "fallback") {
                has_fallback = true;
                fallback_name = Some(func.sig.ident.clone());
            } else if has_pvm_attr(&func.attrs, "method") {
                let param_names: Vec<Ident> = func
                    .sig
                    .inputs
                    .iter()
                    .filter_map(|arg| {
                        if let syn::FnArg::Typed(pat_type) = arg {
                            if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                                return Some(pat_ident.ident.clone());
                            }
                        }
                        None
                    })
                    .collect();

                let param_types: Vec<syn::Type> = func
                    .sig
                    .inputs
                    .iter()
                    .filter_map(|arg| {
                        if let syn::FnArg::Typed(pat_type) = arg {
                            Some((*pat_type.ty).clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                let returns_result = is_result_return_type(&func.sig.output);

                // Extract the raw return type (inner of Result if applicable)
                let return_type = match &func.sig.output {
                    syn::ReturnType::Default => None,
                    syn::ReturnType::Type(_, ty) => {
                        if returns_result {
                            extract_result_ok_type(ty)
                        } else {
                            Some((**ty).clone())
                        }
                    }
                };

                let sol_fn_name = extract_method_rename(&func.attrs)
                    .unwrap_or_else(|| to_snake_case(&func.sig.ident.to_string()));

                let is_sol_interface = sol_interface.is_some();

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
                                    "No matching Solidity function found for `{}` in interface",
                                    sol_fn_name
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

                // For .sol interface path, types are always fully resolved
                let (all_inputs_resolved, return_resolved) = if is_sol_interface {
                    (true, true)
                } else {
                    let inputs_ok = param_types.iter().all(|ty| SolType::from_rust_type(ty).is_some());
                    let return_ok = match &return_type {
                        None => true,
                        Some(ty) => SolType::from_rust_type(ty).is_some(),
                    };
                    (inputs_ok, return_ok)
                };

                methods.push(MethodInfo {
                    fn_name: func.sig.ident.clone(),
                    signature,
                    param_names,
                    param_types,
                    return_type,
                    returns_result,
                    all_inputs_resolved,
                    return_resolved,
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
        has_fallback,
        constructor,
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
    let use_alloc = !args.no_alloc;

    // Determine at macro expansion time whether this crate is the entry point.
    let is_entry_point = match std::env::var("PVM_ENTRY_POINT_CRATE") {
        Ok(entry_crate) => {
            let pkg_name = std::env::var("CARGO_PKG_NAME").unwrap_or_default();
            entry_crate == pkg_name
        }
        Err(_) => false,
    };

    let mod_name = &parsed.mod_name;
    let mod_vis = &input.vis;
    let mod_attrs = &input.attrs;

    let mod_content = strip_pvm_attrs(&input);

    let alloc_setup = if use_alloc {
        let allocator = if is_entry_point {
            super::allocator::generate_allocator(None)
        } else {
            quote! {}
        };
        quote! {
            extern crate alloc;
            use alloc::vec;
            use alloc::vec::Vec;
            #allocator
        }
    } else {
        quote! {}
    };

    let deploy_fn = if let Some(ref ctor) = parsed.constructor {
        let constructor_name = &ctor.fn_name;
        let param_names = &ctor.param_names;

        if ctor.param_names.is_empty() {
            quote! {
                #[unsafe(no_mangle)]
                #[pvm_contract::polkavm_derive::polkavm_export]
                pub extern "C" fn deploy() {
                    match #mod_name::#constructor_name() {
                        Ok(()) => {}
                        Err(e) => {
                            pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, e.as_ref());
                        }
                    }
                }
            }
        } else if ctor.all_resolved {
            let decodes = generate_decode_params(&ctor.param_types, use_alloc);
            let min_size = calculate_min_input_size(&ctor.param_types);
            let decode_stmts: Vec<_> = param_names.iter().zip(decodes.iter())
                .map(|(name, decode)| quote! { let #name = #decode; }).collect();
            let call_args: Vec<_> = param_names.iter().map(|n| quote!(#n)).collect();

            quote! {
                #[unsafe(no_mangle)]
                #[pvm_contract::polkavm_derive::polkavm_export]
                pub extern "C" fn deploy() {
                    let call_data_len = pvm_contract::api::call_data_size() as usize;
                    let mut call_data = vec![0u8; call_data_len];
                    pvm_contract::api::call_data_copy(&mut call_data, 0);
                    let input = &call_data[..];
                    if input.len() < #min_size {
                        pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, b"InvalidConstructorArgs");
                        return;
                    }
                    #(#decode_stmts)*
                    match #mod_name::#constructor_name(#(#call_args),*) {
                        Ok(()) => {}
                        Err(e) => {
                            pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, e.as_ref());
                        }
                    }
                }
            }
        } else {
            // Trait-based constructor decode for unresolved types
            let raw_types = &ctor.raw_param_types;
            let head_size_exprs: Vec<TokenStream> = raw_types.iter().map(|ty| {
                quote! { <#ty as pvm_contract::SolAbi>::SLOT_SIZE }
            }).collect();
            let decode_stmts: Vec<TokenStream> = param_names.iter().zip(raw_types.iter()).map(|(name, ty)| {
                quote! {
                    let #name = <#ty as pvm_contract::SolAbi>::abi_decode(input, __offset);
                    __offset += <#ty as pvm_contract::SolAbi>::SLOT_SIZE;
                }
            }).collect();
            let call_args: Vec<_> = param_names.iter().map(|n| quote!(#n)).collect();

            quote! {
                #[unsafe(no_mangle)]
                #[pvm_contract::polkavm_derive::polkavm_export]
                pub extern "C" fn deploy() {
                    let call_data_len = pvm_contract::api::call_data_size() as usize;
                    let mut call_data = vec![0u8; call_data_len];
                    pvm_contract::api::call_data_copy(&mut call_data, 0);
                    let input = &call_data[..];
                    let __min_size: usize = 0usize #(+ #head_size_exprs)*;
                    if input.len() < __min_size {
                        pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, b"InvalidConstructorArgs");
                        return;
                    }
                    let mut __offset: usize = 0;
                    #(#decode_stmts)*
                    match #mod_name::#constructor_name(#(#call_args),*) {
                        Ok(()) => {}
                        Err(e) => {
                            pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, e.as_ref());
                        }
                    }
                }
            }
        }
    } else {
        quote! {
            #[unsafe(no_mangle)]
            #[pvm_contract::polkavm_derive::polkavm_export]
            pub extern "C" fn deploy() {}
        }
    };

    // Partition methods: static (all resolved) use match arms, trait methods use if-else blocks
    let (static_methods, trait_methods): (Vec<_>, Vec<_>) = parsed.methods.iter()
        .partition(|m| m.all_inputs_resolved && m.return_resolved);

    let static_dispatch_arms: Vec<_> = static_methods
        .iter()
        .map(|m| generate_dispatch_arm(m, mod_name, use_alloc))
        .collect();

    // Methods with resolved inputs but unresolved return also go through match arms
    let mixed_methods: Vec<_> = parsed.methods.iter()
        .filter(|m| m.all_inputs_resolved && !m.return_resolved)
        .collect();
    let mixed_dispatch_arms: Vec<_> = mixed_methods
        .iter()
        .map(|m| generate_dispatch_arm(m, mod_name, use_alloc))
        .collect();

    // Methods with unresolved inputs go through if-else blocks in the _ arm
    let trait_dispatch_blocks: Vec<_> = trait_methods.iter()
        .filter(|m| !m.all_inputs_resolved)
        .map(|m| generate_trait_dispatch_arm(m, mod_name))
        .collect();

    let fallback_call = if parsed.has_fallback {
        let fallback_name = parsed.fallback_name.as_ref().unwrap();
        if use_alloc {
            quote! { #mod_name::#fallback_name().map(|()| None).map_err(|e| e.as_ref().to_vec()) }
        } else {
            quote! { #mod_name::#fallback_name().map(|()| None).map_err(|e| e.as_ref()) }
        }
    } else {
        if use_alloc {
            quote! { Err(Vec::new()) }
        } else {
            quote! { Err(b"") }
        }
    };

    let call_fn = if use_alloc {
        let wildcard_body = if trait_dispatch_blocks.is_empty() {
            quote! { #fallback_call }
        } else {
            quote! {
                #(#trait_dispatch_blocks)*
                #fallback_call
            }
        };

        quote! {
            #[unsafe(no_mangle)]
            #[pvm_contract::polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                let call_data_len = pvm_contract::api::call_data_size() as usize;
                let mut call_data = vec![0u8; call_data_len];
                pvm_contract::api::call_data_copy(&mut call_data, 0);

                let result: Result<Option<Vec<u8>>, Vec<u8>> = (|| {
                    if call_data.len() < 4 {
                        return #fallback_call;
                    }

                    let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                    let input = &call_data[4..];

                    match selector {
                        #(#static_dispatch_arms)*
                        #(#mixed_dispatch_arms)*
                        _ => {
                            #wildcard_body
                        }
                    }
                })();

                match result {
                    Ok(Some(data)) => {
                        pvm_contract::api::return_value(pvm_contract::ReturnFlags::empty(), &data);
                    }
                    Ok(None) => {}
                    Err(data) => {
                        pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, &data);
                    }
                }
            }
        }
    } else {
        let buffer_size = args.buffer_size;
        quote! {
            #[unsafe(no_mangle)]
            #[pvm_contract::polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                let call_data_len = pvm_contract::api::call_data_size() as usize;

                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    <pvm_contract::api as pvm_contract::HostFn>::return_value(pvm_contract::ReturnFlags::REVERT, b"CalldataTooLarge");
                    return;
                }
                pvm_contract::api::call_data_copy(&mut call_data[..call_data_len], 0);

                let result: Result<Option<&[u8]>, &[u8]> = (|| {
                    if call_data_len < 4 {
                        return #fallback_call;
                    }

                    let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                    let input = &call_data[4..call_data_len];

                    match selector {
                        #(#static_dispatch_arms)*
                        #(#mixed_dispatch_arms)*
                        _ => #fallback_call,
                    }
                })();

                match result {
                    Ok(Some(data)) => {
                        pvm_contract::api::return_value(pvm_contract::ReturnFlags::empty(), data);
                    }
                    Ok(None) => {}
                    Err(data) => {
                        pvm_contract::api::return_value(pvm_contract::ReturnFlags::REVERT, data);
                    }
                }
            }
        }
    };

    let error_enum = quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Error {}

        impl AsRef<[u8]> for Error {
            fn as_ref(&self) -> &[u8] {
                match *self {}
            }
        }
    };

    let reference_type = generate_reference_type(&parsed.methods, args.cdm.as_deref());
    let abi_section = generate_abi_section(&parsed, sol_interface.as_ref());

    let cdm_section = args.cdm.as_deref().map(|name| generate_cdm_section(name, is_entry_point));

    if is_entry_point {
        Ok(quote! {
            use pvm_contract::HostFn as _;

            #alloc_setup

            #deploy_fn

            #call_fn

            #abi_section

            #cdm_section

            #(#mod_attrs)*
            #mod_vis mod #mod_name {
                #error_enum

                #mod_content
            }

            #reference_type
        })
    } else {
        // Not the entry point — only emit the module content and reference type.
        // Deploy/call functions, ABI, and CDM are only needed in the entry-point binary.
        Ok(quote! {
            use pvm_contract::HostFn as _;

            #alloc_setup

            #(#mod_attrs)*
            #mod_vis mod #mod_name {
                #error_enum

                #mod_content
            }

            #reference_type
        })
    }
}

fn generate_abi_section(parsed: &ParsedContract, sol_interface: Option<&SolInterface>) -> TokenStream {
    let is_sol = sol_interface.is_some();

    // Build a flat list of concatcp expressions. Each element is a single expr
    // (string literal or trait constant). This avoids nested quote! splicing issues.
    let mut parts: Vec<TokenStream> = Vec::new();
    parts.push(quote! { "[" });

    let mut first_entry = true;

    // Constructor ABI entry
    if let Some(ref ctor) = parsed.constructor {
        first_entry = false;
        parts.push(quote! { "{\"type\":\"constructor\",\"inputs\":[" });
        push_abi_params(&mut parts, &ctor.param_names, &ctor.raw_param_types,
            if ctor.all_resolved { Some(&ctor.param_types) } else { None },
            ctor.all_resolved, &ctor.param_types);
        parts.push(quote! { "],\"stateMutability\":\"nonpayable\"}" });
    }

    // Method ABI entries
    for method in &parsed.methods {
        if !first_entry {
            parts.push(quote! { "," });
        }
        first_entry = false;

        let method_name = &method.signature.name;
        let mutability = if method.return_type.is_some() { "view" } else { "nonpayable" };

        parts.push(quote! { "{\"type\":\"function\",\"name\":\"" });
        parts.push(quote! { #method_name });
        parts.push(quote! { "\",\"inputs\":[" });

        push_abi_params(&mut parts, &method.param_names, &method.param_types,
            if is_sol { Some(&method.signature.inputs) } else { None },
            is_sol, &method.signature.inputs);

        parts.push(quote! { "],\"outputs\":[" });

        push_abi_outputs(&mut parts, &method.return_type,
            if is_sol { &method.signature.outputs } else { &[] }, is_sol);

        parts.push(quote! { "],\"stateMutability\":\"" });
        parts.push(quote! { #mutability });
        parts.push(quote! { "\"}" });
    }

    parts.push(quote! { "]" });

    // ABI metadata is only extracted from the entry-point ELF binary.
    // The caller already handles the entry-point vs dependency split.
    quote! {
        const __PVM_ABI_JSON: &str = pvm_contract::const_format::concatcp!(
            #(#parts),*
        );

        #[unsafe(link_section = ".rodata.pvm_abi")]
        #[unsafe(no_mangle)]
        #[used]
        static __PVM_ABI: [u8; __PVM_ABI_JSON.len()] = {
            let b = __PVM_ABI_JSON.as_bytes();
            let mut a = [0u8; __PVM_ABI_JSON.len()];
            let mut i = 0;
            while i < b.len() {
                a[i] = b[i];
                i += 1;
            }
            a
        };
    }
}

fn generate_cdm_section(cdm_name: &str, is_entry_point: bool) -> TokenStream {
    // CDM metadata is only extracted from the entry-point ELF binary.
    // No reason to emit it when compiled as a library dependency.
    if !is_entry_point {
        return quote! {};
    }

    quote! {
        const __PVM_CDM_JSON: &str = pvm_contract::const_format::concatcp!(
            "{\"cdmPackage\":\"", #cdm_name, "\"}"
        );

        #[unsafe(link_section = ".rodata.pvm_cdm")]
        #[unsafe(no_mangle)]
        #[used]
        static __PVM_CDM: [u8; __PVM_CDM_JSON.len()] = {
            let b = __PVM_CDM_JSON.as_bytes();
            let mut a = [0u8; __PVM_CDM_JSON.len()];
            let mut i = 0;
            while i < b.len() {
                a[i] = b[i];
                i += 1;
            }
            a
        };
    }
}

/// Push ABI type and components tokens for a Rust type, with special handling
/// for `Option<T>` (which needs inline component generation since the blanket
/// `SolAbi for Option<T>` impl cannot produce `ABI_COMPONENTS` via `concatcp!`).
fn push_abi_type_and_components(parts: &mut Vec<TokenStream>, ty: &syn::Type) {
    if let Some(inner) = unwrap_option_inner(ty) {
        parts.push(quote! { "tuple" });
        parts.push(quote! { "\"" });
        parts.push(quote! {
            pvm_contract::const_format::concatcp!(
                ",\"components\":[{\"name\":\"isSome\",\"type\":\"bool\"},{\"name\":\"value\",\"type\":\"",
                <#inner as pvm_contract::SolAbi>::ABI_TYPE,
                "\"",
                <#inner as pvm_contract::SolAbi>::ABI_COMPONENTS,
                "}]"
            )
        });
    } else {
        parts.push(quote! { <#ty as pvm_contract::SolAbi>::ABI_TYPE });
        parts.push(quote! { "\"" });
        parts.push(quote! { <#ty as pvm_contract::SolAbi>::ABI_COMPONENTS });
    }
}

/// Push ABI parameter expressions into the flat parts list.
fn push_abi_params(
    parts: &mut Vec<TokenStream>,
    param_names: &[Ident],
    rust_types: &[syn::Type],
    sol_types_opt: Option<&[SolType]>,
    is_sol: bool,
    sol_types_for_sol: &[SolType],
) {
    for (i, name) in param_names.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        let name_str = name.to_string();

        if is_sol {
            let sol_type = sol_types_opt
                .and_then(|s| s.get(i))
                .or_else(|| sol_types_for_sol.get(i));
            if let Some(sol_type) = sol_type {
                let type_str = sol_type.canonical_name();
                parts.push(quote! { "{\"name\":\"" });
                parts.push(quote! { #name_str });
                parts.push(quote! { "\",\"type\":\"" });
                parts.push(quote! { #type_str });
                parts.push(quote! { "\"" });
                push_sol_type_components(parts, sol_type);
                parts.push(quote! { "}" });
            }
        } else {
            let ty = &rust_types[i];
            parts.push(quote! { "{\"name\":\"" });
            parts.push(quote! { #name_str });
            parts.push(quote! { "\",\"type\":\"" });
            push_abi_type_and_components(parts, ty);
            parts.push(quote! { "}" });
        }
    }
}

/// Push ABI output expressions into the flat parts list.
fn push_abi_outputs(
    parts: &mut Vec<TokenStream>,
    return_type: &Option<syn::Type>,
    sol_outputs: &[SolType],
    is_sol: bool,
) {
    if is_sol {
        for (i, sol_type) in sol_outputs.iter().enumerate() {
            if i > 0 {
                parts.push(quote! { "," });
            }
            let type_str = sol_type.canonical_name();
            parts.push(quote! { "{\"name\":\"\"" });
            parts.push(quote! { ",\"type\":\"" });
            parts.push(quote! { #type_str });
            parts.push(quote! { "\"" });
            push_sol_type_components(parts, sol_type);
            parts.push(quote! { "}" });
        }
    } else if let Some(ty) = return_type {
        parts.push(quote! { "{\"name\":\"\"" });
        parts.push(quote! { ",\"type\":\"" });
        push_abi_type_and_components(parts, ty);
        parts.push(quote! { "}" });
    }
}

/// Push JSON components fragment for a SolType (recursive for nested tuples).
fn push_sol_type_components(parts: &mut Vec<TokenStream>, sol_type: &SolType) {
    if let SolType::Tuple(fields) = sol_type {
        parts.push(quote! { ",\"components\":[" });
        for (i, field) in fields.iter().enumerate() {
            if i > 0 {
                parts.push(quote! { "," });
            }
            let field_name = format!("_field{}", i);
            let field_type = field.canonical_name();
            parts.push(quote! { "{\"name\":\"" });
            parts.push(quote! { #field_name });
            parts.push(quote! { "\",\"type\":\"" });
            parts.push(quote! { #field_type });
            parts.push(quote! { "\"" });
            push_sol_type_components(parts, field);
            parts.push(quote! { "}" });
        }
        parts.push(quote! { "]" });
    }
}

/// Generate JSON components fragment for a SolType (used in .sol interface mode).

fn generate_reference_method(method: &MethodInfo) -> TokenStream {
    let fn_name = &method.fn_name;
    let fully_resolved = method.all_inputs_resolved && method.return_resolved;
    let sig = &method.signature;

    // --- Varying piece 1: function params ---
    let params: Vec<TokenStream> = if fully_resolved {
        method.param_names.iter().zip(sig.inputs.iter())
            .map(|(name, ty)| { let rt = ty.rust_type(true); quote! { #name: #rt } }).collect()
    } else {
        method.param_names.iter().zip(method.param_types.iter())
            .map(|(name, ty)| quote! { #name: #ty }).collect()
    };

    // --- Varying piece 2: selector + calldata init ---
    let selector_setup = if fully_resolved {
        let [s0, s1, s2, s3] = compute_selector(&sig.canonical_signature());
        quote! { let mut calldata = alloc::vec![#s0, #s1, #s2, #s3]; }
    } else {
        let sol_name = &sig.name;
        let param_types = &method.param_types;
        let sol_name_exprs: Vec<TokenStream> = param_types.iter().map(|ty| {
            quote! { <#ty as pvm_contract::SolAbi>::SOL_NAME }
        }).collect();
        quote! {
            let __sel = pvm_contract::compute_selector(#sol_name, &[
                #(#sol_name_exprs),*
            ]);
            let mut calldata = alloc::vec![__sel[0], __sel[1], __sel[2], __sel[3]];
        }
    };

    // --- Varying piece 3: encode statements ---
    let encode_block: TokenStream = if fully_resolved {
        generate_encode_params(&method.param_names, &sig.inputs)
    } else {
        generate_encode_params_trait(&method.param_names, &method.param_types)
    };

    // --- Varying pieces 4-6: return type, output buffer, decode ---
    let (ret_ty, decode_return, has_output) = if fully_resolved {
        generate_resolved_return_parts(&sig.outputs)
    } else {
        match &method.return_type {
            None => (quote! { () }, quote! { () }, false),
            Some(ty) => (
                quote! { #ty },
                quote! { <#ty as pvm_contract::SolAbi>::abi_decode(output, 0) },
                true,
            ),
        }
    };

    let success_body = if has_output {
        quote! { Ok(#decode_return) }
    } else {
        quote! { Ok(()) }
    };

    // --- Single function body template ---
    quote! {
        pub fn #fn_name(&self, #(#params),*) -> pvm_contract::call::CallResult<#ret_ty> {
            extern crate alloc;
            #selector_setup
            #encode_block
            let output = pvm_contract::call::call_evm_collect(
                pvm_contract::CallFlags::ALLOW_REENTRY,
                self.addr.as_fixed_bytes(), u64::MAX, &[0u8; 32], &calldata,
            )?;
            #success_body
        }
    }
}

fn generate_reference_type(methods: &[MethodInfo], cdm: Option<&str>) -> TokenStream {
    let proxy_methods: Vec<TokenStream> = methods.iter().map(generate_reference_method).collect();

    let cdm_fn = if let Some(cdm_name) = cdm {
        generate_cdm_reference(cdm_name)
    } else {
        quote! {}
    };

    quote! {
        #[derive(pvm_contract::Encode, pvm_contract::Decode)]
        pub struct Reference {
            addr: pvm_contract::Address,
        }

        impl Reference {
            pub fn at(addr: pvm_contract::Address) -> Self { Self { addr } }
            pub fn address(&self) -> &pvm_contract::Address { &self.addr }
            #(#proxy_methods)*
        }

        pub fn reference(addr: pvm_contract::Address) -> Reference {
            Reference::at(addr)
        }

        #cdm_fn
    }
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
                        && segments[0].ident == "pvm"
                        && (segments[1].ident == "method"
                            || segments[1].ident == "constructor"
                            || segments[1].ident == "fallback"))
                });
                quote! { #new_func }
            }
            syn::Item::Struct(s) if has_pvm_attr(&s.attrs, "storage") => {
                match super::expand_storage(s) {
                    Ok(tokens) => tokens,
                    Err(err) => err.to_compile_error(),
                }
            }
            other => quote! { #other },
        })
        .collect();

    quote! {
        #[allow(unused_imports)]
        use pvm_contract::HostFn as _;

        #(#items)*
    }
}
