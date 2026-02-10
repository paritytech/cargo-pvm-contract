use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse::ParseStream, Attribute, Ident, ItemMod, LitInt, LitStr, Token};

use super::dispatch::{generate_dispatch_arm, MethodInfo};
use crate::signature::{FunctionSignature, SolType};
use crate::solidity::{parse_solidity_interface, to_snake_case, SolInterface};

pub struct ContractArgs {
    pub no_alloc: bool,
    pub buffer_size: usize,
    pub sol_path: Option<String>,
}

impl Default for ContractArgs {
    fn default() -> Self {
        ContractArgs {
            no_alloc: false,
            buffer_size: 256,
            sol_path: None,
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

struct ParsedContract {
    mod_name: Ident,
    methods: Vec<MethodInfo>,
    has_constructor: bool,
    has_fallback: bool,
    constructor_name: Option<Ident>,
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
    let mut has_constructor = false;
    let mut has_fallback = false;
    let mut constructor_name = None;
    let mut fallback_name = None;
    let mut implemented_sol_methods = Vec::new();

    for item in &content.1 {
        if let syn::Item::Fn(func) = item {
            if has_pvm_attr(&func.attrs, "constructor") {
                has_constructor = true;
                constructor_name = Some(func.sig.ident.clone());
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

    let mod_name = &parsed.mod_name;
    let mod_vis = &input.vis;
    let mod_attrs = &input.attrs;

    let mod_content = strip_pvm_attrs(&input);

    let alloc_setup = if use_alloc {
        quote! {
            extern crate alloc;
            use alloc::vec;
            use alloc::vec::Vec;
        }
    } else {
        quote! {}
    };

    let panic_handler = quote! {
        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
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
    } else {
        quote! {
            #[unsafe(no_mangle)]
            #[pvm_contract::polkavm_derive::polkavm_export]
            pub extern "C" fn deploy() {}
        }
    };

    let dispatch_arms: Vec<_> = parsed
        .methods
        .iter()
        .map(|m| generate_dispatch_arm(m, mod_name, use_alloc))
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
                        #(#dispatch_arms)*
                        _ => #fallback_call,
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
                        #(#dispatch_arms)*
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

    Ok(quote! {
        use pvm_contract::HostFn as _;

        #alloc_setup

        #panic_handler

        #deploy_fn

        #call_fn

        #(#mod_attrs)*
        #mod_vis mod #mod_name {
            #error_enum

            #mod_content
        }
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
                        && segments[0].ident == "pvm"
                        && (segments[1].ident == "method"
                            || segments[1].ident == "constructor"
                            || segments[1].ident == "fallback"))
                });
                quote! { #new_func }
            }
            syn::Item::Struct(s) if has_pvm_storage_attr(&s.attrs) => {
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

fn has_pvm_storage_attr(attrs: &[Attribute]) -> bool {
    for attr in attrs {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        if segments.len() == 2 && segments[0].ident == "pvm" && segments[1].ident == "storage" {
            return true;
        }
    }
    false
}

