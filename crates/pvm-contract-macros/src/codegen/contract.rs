use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Ident, ItemMod, LitInt, LitStr, Token, parse::Parse, parse::ParseStream};
use syn_solidity::Item;

use super::abi_gen::generate_abi_gen;
use super::dispatch::{
    MethodInfo, RouteItems, boundary_size_check, generate_param_decoding,
    generate_revert_encoding_boundary, generate_router,
};
use crate::signature::{SolType, compute_selector};
use crate::utils::{compute_function_signature, to_snake_case};

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

fn load_sol_interface(path: &str) -> Result<syn_solidity::File, String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "CARGO_MANIFEST_DIR not set".to_string())?;
    let full_path = std::path::Path::new(&manifest_dir).join(path);
    let source = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("Failed to read {}: {}", full_path.display(), e))?;
    syn::parse_str(&source)
        .and_then(syn_solidity::parse2)
        .map_err(|e| format!("Failed to read {}: {}", full_path.display(), e))
}

pub(super) struct ParsedContract {
    /// Module name wrapping the contract (e.g. `my_token`).
    pub(super) mod_name: Ident,
    /// Contract struct name (e.g. `MyToken`). None if the module uses the
    /// legacy free-function form — the expander errors in that case.
    pub(super) struct_name: Option<Ident>,
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

const VALID_PREFIXES: &[&str] = &[
    "pvm",
    "pvm_contract",
    "pvm_contract_macros",
    "pvm_contract_sdk",
];

fn check_signature_compatibility(
    func: &syn::ImplItemFn,
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
            continue;
        };
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
        if segments.len() == 1 && segments[0].ident == name {
            return true;
        }
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

fn collect_error_type(
    output: &syn::ReturnType,
    error_types: &mut Vec<syn::Type>,
    seen_error_names: &mut Vec<String>,
) {
    if let syn::ReturnType::Type(_, ty) = output
        && let syn::Type::Path(type_path) = ty.as_ref()
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
    {
        let type_args: Vec<_> = args
            .args
            .iter()
            .filter_map(|a| {
                if let syn::GenericArgument::Type(t) = a {
                    Some(t)
                } else {
                    None
                }
            })
            .collect();
        if type_args.len() >= 2 {
            let error_ty = type_args[1].clone();
            let name = quote! { #error_ty }.to_string();
            if !seen_error_names.contains(&name) {
                seen_error_names.push(name);
                error_types.push(error_ty);
            }
        }
    }
}

fn to_camel_case(snake: &str) -> String {
    let mut result = String::new();
    let mut next_upper = false;
    for (i, c) in snake.chars().enumerate() {
        if c == '_' {
            next_upper = true;
        } else if i == 0 {
            result.push(c);
        } else if next_upper {
            result.push(c.to_ascii_uppercase());
            next_upper = false;
        } else {
            result.push(c);
        }
    }
    result
}

fn extract_return_types(output: &syn::ReturnType) -> Vec<syn::Type> {
    match output {
        syn::ReturnType::Default => vec![],
        syn::ReturnType::Type(_, ty) => {
            if is_result_return_type(output) {
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

/// Extract typed params from an impl-method's `FnArg` list.
///
/// Requires the first parameter to be a `self` receiver (`&self`, `&mut self`,
/// or owned `self`) — without one, dispatch can't call `this.method(...)` on
/// the generated contract struct, so we error loudly here instead of producing
/// a cryptic "method not found" error from expanded code.
fn extract_typed_params_impl(
    func: &syn::ImplItemFn,
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
) -> syn::Result<Vec<(Ident, syn::Type)>> {
    let Some(first) = inputs.first() else {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "Contract methods must take `&self` or `&mut self` as the first parameter",
        ));
    };
    match first {
        syn::FnArg::Receiver(r) if r.reference.is_none() => {
            return Err(syn::Error::new_spanned(
                r,
                "Contract methods must take a borrowed self (`&self` / `&mut self`); owning `self` would consume the contract instance",
            ));
        }
        syn::FnArg::Receiver(_) => {}
        syn::FnArg::Typed(_) => {
            return Err(syn::Error::new_spanned(
                first,
                "Contract methods must take `&self` or `&mut self` as the first parameter",
            ));
        }
    }

    inputs
        .iter()
        .skip(1)
        .map(|arg| match arg {
            syn::FnArg::Receiver(r) => Err(syn::Error::new_spanned(
                r,
                "Only the first parameter may be a `self` receiver",
            )),
            syn::FnArg::Typed(pat_type) => {
                let ident = if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    pat_ident.ident.clone()
                } else {
                    return Err(syn::Error::new_spanned(
                        &pat_type.pat,
                        "Parameters must be simple identifiers",
                    ));
                };
                Ok((ident, (*pat_type.ty).clone()))
            }
        })
        .collect()
}

fn parse_contract(
    input: &ItemMod,
    sol_interface: Option<&syn_solidity::File>,
) -> syn::Result<ParsedContract> {
    let mod_name = input.ident.clone();
    let content = input
        .content
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(input, "Contract module must have a body"))?;

    // The contract struct is the self-type of the first `impl` block that
    // contains `#[method]` / `#[constructor]` / `#[fallback]` methods.
    let struct_name = content.1.iter().find_map(|item| {
        let syn::Item::Impl(item_impl) = item else {
            return None;
        };
        let has_contract_attrs = item_impl.items.iter().any(|ii| {
            if let syn::ImplItem::Fn(f) = ii {
                has_pvm_attr(&f.attrs, "method")
                    || has_pvm_attr(&f.attrs, "constructor")
                    || has_pvm_attr(&f.attrs, "fallback")
            } else {
                false
            }
        });
        if !has_contract_attrs {
            return None;
        }
        // Extract the struct ident from `impl<G> StructName<G> { ... }`
        let syn::Type::Path(type_path) = item_impl.self_ty.as_ref() else {
            return None;
        };
        type_path.path.segments.last().map(|s| s.ident.clone())
    });

    // A contract is a single type — the dispatcher constructs one `this` and
    // calls every method on it. Methods scattered across different structs
    // would either fail with a confusing "no method" error from macro-generated
    // code, or silently dispatch to a same-named method on the wrong type.
    if let Some(expected) = &struct_name {
        for item in &content.1 {
            // Reject generics on the contract struct itself. Contract methods
            // are dispatched by 4-byte Solidity selectors (keccak of canonical
            // signatures), which require concrete types. Generic contract types
            // are not yet supported; for composability, prefer trait impls with
            // concrete type arguments at the impl site.
            if let syn::Item::Struct(item_struct) = item
                && &item_struct.ident == expected
                && !item_struct.generics.params.is_empty()
            {
                return Err(syn::Error::new_spanned(
                    &item_struct.generics.params,
                    "contract structs must not be generic",
                ));
            }

            let syn::Item::Impl(item_impl) = item else {
                continue;
            };
            let has_contract_attrs = item_impl.items.iter().any(|ii| {
                if let syn::ImplItem::Fn(f) = ii {
                    has_pvm_attr(&f.attrs, "method")
                        || has_pvm_attr(&f.attrs, "constructor")
                        || has_pvm_attr(&f.attrs, "fallback")
                } else {
                    false
                }
            });
            if !has_contract_attrs {
                continue;
            }
            let ident = match item_impl.self_ty.as_ref() {
                syn::Type::Path(type_path) => type_path.path.segments.last().map(|s| &s.ident),
                _ => None,
            };
            let Some(ident) = ident else {
                return Err(syn::Error::new_spanned(
                    &item_impl.self_ty,
                    "Contract `impl` target must be a named struct type",
                ));
            };
            if ident != expected {
                return Err(syn::Error::new_spanned(
                    &item_impl.self_ty,
                    format!(
                        "All contract `impl` blocks must target the same struct; expected `{expected}`, found `{ident}`"
                    ),
                ));
            }

            // Reject generics on the contract `impl` block.
            if !item_impl.generics.params.is_empty() {
                return Err(syn::Error::new_spanned(
                    &item_impl.generics.params,
                    "contract `impl` blocks must not be generic",
                ));
            }

            // Reject generics on contract methods — selectors are concrete.
            for impl_item in &item_impl.items {
                let syn::ImplItem::Fn(func) = impl_item else {
                    continue;
                };
                if !(has_pvm_attr(&func.attrs, "method")
                    || has_pvm_attr(&func.attrs, "constructor")
                    || has_pvm_attr(&func.attrs, "fallback"))
                {
                    continue;
                }
                if !func.sig.generics.params.is_empty() {
                    return Err(syn::Error::new_spanned(
                        &func.sig.generics.params,
                        "contract methods must not be generic",
                    ));
                }
            }
        }
    }

    // Collect methods from every `impl` block in the module.
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
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        for impl_item in &item_impl.items {
            let syn::ImplItem::Fn(func) = impl_item else {
                continue;
            };

            if has_pvm_attr(&func.attrs, "constructor") {
                has_constructor = true;
                constructor_name = Some(func.sig.ident.clone());
                constructor_returns_result = is_result_return_type(&func.sig.output);
                constructor_inputs = extract_typed_params_impl(func, &func.sig.inputs)?;
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
            } else if has_pvm_attr(&func.attrs, "fallback") {
                has_fallback = true;
                fallback_name = Some(func.sig.ident.clone());
                fallback_returns_result = is_result_return_type(&func.sig.output);
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
            } else if has_pvm_attr(&func.attrs, "method") {
                let typed_params = extract_typed_params_impl(func, &func.sig.inputs)?;
                let param_names: Vec<Ident> = typed_params.iter().map(|(n, _)| n.clone()).collect();
                let param_types: Vec<syn::Type> =
                    typed_params.into_iter().map(|(_, t)| t).collect();

                let returns_result = is_result_return_type(&func.sig.output);
                let return_types = extract_return_types(&func.sig.output);

                let (sol_name, precomputed_selector) = if let Some(sol_iface) = sol_interface
                    && let Some(sol_iface) = {
                        let mut items = sol_iface.items.iter().filter_map(|x| match x {
                            Item::Contract(item_contract) if item_contract.is_interface() => {
                                Some(item_contract)
                            }
                            _ => None,
                        });
                        if let i_face @ Some(_) = items.next()
                            && items.next().is_none()
                        {
                            i_face
                        } else {
                            return Err(syn::Error::new_spanned(
                                input,
                                "Only one contract interface per file is supported",
                            ));
                        }
                    } {
                    let rust_fn_name = func.sig.ident.to_string();
                    let rename = extract_method_rename(&func.attrs)?
                        .unwrap_or_else(|| to_snake_case(&rust_fn_name));
                    let sol_func = sol_iface
                        .body.iter().find_map(|f| match f {
                            syn_solidity::Item::Function(item_function)  if item_function.name.as_ref().is_some_and(|name| name.as_string() == rename || to_snake_case(name.to_string().as_str()) == rust_fn_name) => Some(item_function),
                           _ => None
                        })
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                func,
                                format!(
                                    "No matching Solidity function found for `{rename}` in interface"
                                ),
                            )
                        })?;
                    let sig = sol_func
                        .parameters
                        .types()
                        .map(|x| x.clone().try_into())
                        .collect::<Result<Vec<SolType>, String>>();
                    check_signature_compatibility(
                        func,
                        &sol_func.name().to_string(),
                        &sig.map_err(|x| {
                            syn::Error::new_spanned(
                                func,
                                format!(
                                    "Failed to map syn_solidity abstraction `{x}` to supported type in interface"
                                ),
                            )
                        })?,
                        &param_types,
                    )?;
                    implemented_sol_methods.push(sol_func.name.clone());
                    let selector = compute_selector(&compute_function_signature(sol_func));
                    (sol_func.name().to_string(), Some(selector))
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

    if let Some(sol_iface) = sol_interface
        && let Some(sol_iface) = {
            let mut items = sol_iface.items.iter().filter_map(|x| match x {
                Item::Contract(item_contract) if item_contract.is_interface() => {
                    Some(item_contract)
                }
                _ => None,
            });
            if let i_face @ Some(_) = items.next()
                && items.next().is_none()
            {
                i_face
            } else {
                return Err(syn::Error::new_spanned(
                    input,
                    "Only one contract interface per file is supported",
                ));
            }
        }
    {
        let missing: Vec<_> = sol_iface
            .body
            .iter()
            .filter_map(|f| match f {
                syn_solidity::Item::Function(item_function) => Some(item_function),
                _ => None,
            })
            .filter(|f| !implemented_sol_methods.contains(&f.name))
            .map(|f| f.name().to_string())
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
        struct_name,
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
    let storage_struct_name = find_sol_storage_struct(&input)?;
    let (abi_gen_helper, abi_gen_main) = generate_abi_gen(
        &parsed,
        args.sol_path.is_some(),
        storage_struct_name.clone(),
    );

    let mod_name = &parsed.mod_name;
    let mod_vis = &input.vis;
    let mod_attrs = &input.attrs;

    let struct_name = parsed.struct_name.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(
            &input,
            "Contract module must contain a storage struct (e.g. `pub struct Foo;`)",
        )
    })?;

    let mod_content = strip_pvm_attrs(&input, struct_name, &storage_struct_name)?;

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

    let buffer_size = args.buffer_size;

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
            min_size_expr,
            decode_statements,
            call_args,
            has_params,
        } = decoding;
        let size_check = boundary_size_check(has_params, &min_size_expr);

        let read_calldata = if param_names.is_empty() {
            quote! {}
        } else if use_alloc {
            quote! {
                let call_data_len = ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = alloc::vec![0u8; call_data_len];
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data, 0);
                let input = &call_data[..];
                #size_check
            }
        } else {
            quote! {
                let call_data_len = ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                        ::pvm_contract_sdk::ReturnFlags::REVERT,
                        &::pvm_contract_sdk::framework_errors::CALLDATA_TOO_LARGE);
                }
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data[..call_data_len], 0);
                let input = &call_data[..call_data_len];
                #size_check
            }
        };

        let call_expr = quote! { this.#constructor_name(#(#call_args),*) };
        let revert_err = generate_revert_encoding_boundary(use_alloc);
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
            #[cfg(target_arch = "riscv64")]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn deploy() {
                use ::pvm_contract_sdk::pallet_revive_uapi::HostFn as _;
                let mut this = #struct_name {
                    host: ::pvm_contract_sdk::Host::new(),
                };
                #read_calldata
                #decode_and_call
            }
        }
    } else {
        quote! {
            #[cfg(target_arch = "riscv64")]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn deploy() {}
        }
    };

    let (route_items, router_impl) =
        generate_router(&parsed.methods, mod_name, struct_name, use_alloc);
    let RouteItems { route_fn } = route_items;
    let router_impl = router_impl.tokens;

    let (no_selector_handler, unknown_selector_handler) = if parsed.has_fallback {
        let fallback_name = parsed.fallback_name.as_ref().unwrap();
        let handler = if parsed.fallback_returns_result {
            let revert_err = generate_revert_encoding_boundary(use_alloc);
            quote! {
                match this.#fallback_name() {
                    Ok(()) => return,
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        } else {
            quote! {
                this.#fallback_name();
                return;
            }
        };
        (handler.clone(), handler)
    } else {
        (
            quote! {
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                    ::pvm_contract_sdk::ReturnFlags::REVERT,
                    &::pvm_contract_sdk::framework_errors::NO_SELECTOR);
            },
            quote! {
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                    ::pvm_contract_sdk::ReturnFlags::REVERT,
                    &::pvm_contract_sdk::framework_errors::UNKNOWN_SELECTOR);
            },
        )
    };

    // `call()` is the riscv64 boundary: read calldata, dispatch via `route()`.
    // Each matched dispatch arm calls `host.return_value(...)` directly
    // (diverges via syscall) — no buffer round-trip, no result enum to
    // translate. If `route()` returns `None`, no selector matched and we
    // fall through to the fallback or unknown-selector handler.
    let call_fn = if use_alloc {
        quote! {
            #[cfg(target_arch = "riscv64")]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                use ::pvm_contract_sdk::pallet_revive_uapi::HostFn as _;
                let mut this = #struct_name {
                    host: ::pvm_contract_sdk::Host::new(),
                };
                let call_data_len = ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = alloc::vec![0u8; call_data_len];
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data, 0);

                if call_data_len < 4 {
                    #no_selector_handler
                }

                let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                let input = &call_data[4..];

                if route(&mut this, selector, input).is_none() {
                    #unknown_selector_handler
                }
            }
        }
    } else {
        quote! {
            #[cfg(target_arch = "riscv64")]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                use ::pvm_contract_sdk::pallet_revive_uapi::HostFn as _;
                let mut this = #struct_name {
                    host: ::pvm_contract_sdk::Host::new(),
                };
                let call_data_len = ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                        ::pvm_contract_sdk::ReturnFlags::REVERT,
                        &::pvm_contract_sdk::framework_errors::CALLDATA_TOO_LARGE);
                }
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data[..call_data_len], 0);

                if call_data_len < 4 {
                    #no_selector_handler
                }

                let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                let input = &call_data[4..call_data_len];

                if route(&mut this, selector, input).is_none() {
                    #unknown_selector_handler
                }
            }
        }
    };

    Ok(quote! {
        #alloc_setup

        #panic_handler

        #(#mod_attrs)*
        #mod_vis mod #mod_name {
            #mod_content

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

/// Rewrite the contract module body:
/// - Inject a `host: ::pvm_contract_sdk::Host` field on the storage struct.
/// - Strip `#[method]` / `#[constructor]` / `#[fallback]` attrs from methods.
/// - Emit an `impl StorageStruct { fn host(&self) -> &Host }` accessor.
/// - If a `#[derive(SolStorage)]` struct is present in the module, inject
///   `let mut storage = <StorageStruct as SolStorage>::__pvm_storage(self.host().clone());`
///   at the top of every `#[method]`/`#[constructor]`/`#[fallback]` body, so
///   contract code can read/write state via `storage.<field>.get()` etc.
///
/// All user `impl` blocks are cfg-gated to `not(feature = "abi-gen")` so their
/// bodies (which may call host APIs) are excluded from host-target ABI builds.
fn strip_pvm_attrs(
    input: &ItemMod,
    struct_name: &Ident,
    storage_struct_name: &Option<Ident>,
) -> syn::Result<TokenStream> {
    let content = input.content.as_ref().unwrap();
    let mut items: Vec<TokenStream> = Vec::new();
    let mut struct_seen = false;

    for item in &content.1 {
        match item {
            syn::Item::Struct(item_struct) if &item_struct.ident == struct_name => {
                struct_seen = true;
                let rewritten = rewrite_storage_struct(item_struct)?;
                items.push(rewritten);
            }
            syn::Item::Impl(item_impl) => {
                let mut new_impl = item_impl.clone();
                let targets_contract = impl_targets_storage_struct(&new_impl, struct_name);
                for impl_item in new_impl.items.iter_mut() {
                    if let syn::ImplItem::Fn(func) = impl_item {
                        let is_pvm_fn = has_pvm_method_attr(&func.attrs);
                        func.attrs.retain(|attr| {
                            let segments: Vec<_> = attr.path().segments.iter().collect();
                            !(segments.len() == 2
                                && VALID_PREFIXES.contains(&segments[0].ident.to_string().as_str())
                                && (segments[1].ident == "method"
                                    || segments[1].ident == "constructor"
                                    || segments[1].ident == "fallback"))
                        });
                        // Inject `let mut storage = ...` only into pvm-tagged
                        // methods on the contract struct, when a SolStorage
                        // struct is declared in the module.
                        if is_pvm_fn
                            && targets_contract
                            && let Some(storage_ident) = storage_struct_name
                        {
                            let injection: syn::Stmt = syn::parse_quote! {
                                let mut storage = <#storage_ident as ::pvm_contract_sdk::SolStorage>::__pvm_storage(self.host().clone());
                            };
                            func.block.stmts.insert(0, injection);
                        }
                    }
                }
                items.push(quote! {
                    #[cfg(not(feature = "abi-gen"))]
                    #new_impl
                });
            }
            syn::Item::Use(use_item) => {
                let use_str = quote! { #use_item }.to_string();
                if use_str.contains("alloc ::") || use_str.contains("alloc::") {
                    items.push(quote! {
                        #[cfg(not(feature = "abi-gen"))]
                        #use_item
                    });
                } else {
                    items.push(quote! { #use_item });
                }
            }
            other => items.push(quote! { #other }),
        }
    }

    if !struct_seen {
        return Err(syn::Error::new_spanned(
            input,
            format!(
                "Storage struct `{struct_name}` declaration not found in contract module. \
                 Declare it as `pub struct {struct_name};` (the macro injects the `host` field).",
            ),
        ));
    }

    // Inject the `host()` accessor. The generated struct has a private `host`
    // field; contract method bodies reach the host via `self.host()`, mirroring
    // Stylus's `self.vm()` and ink!'s `self.env()`.
    let host_accessor = quote! {
        #[cfg(not(feature = "abi-gen"))]
        impl #struct_name {
            #[inline(always)]
            pub fn host(&self) -> &::pvm_contract_sdk::Host {
                &self.host
            }
        }
    };

    Ok(quote! {
        #[cfg(not(feature = "abi-gen"))]
        #[allow(unused_imports)]
        use ::pvm_contract_sdk::HostApi as _;

        #(#items)*

        #host_accessor
    })
}

/// Whether any of the attributes is a `#[method]`, `#[constructor]`, or
/// `#[fallback]` from one of the pvm crates. Used to decide where to inject
/// the `storage` binding.
fn has_pvm_method_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let segments: Vec<_> = attr.path().segments.iter().collect();
        segments.len() == 2
            && VALID_PREFIXES.contains(&segments[0].ident.to_string().as_str())
            && (segments[1].ident == "method"
                || segments[1].ident == "constructor"
                || segments[1].ident == "fallback")
    })
}

/// Rewrite a user-declared storage struct into `pub struct Name { host: Host, <user fields> }`.
/// Accepts unit (`pub struct Name;`) or named (`pub struct Name { ... }`) forms.
fn rewrite_storage_struct(item_struct: &syn::ItemStruct) -> syn::Result<TokenStream> {
    let attrs = &item_struct.attrs;
    let vis = &item_struct.vis;
    let name = &item_struct.ident;

    // Drop any user-declared `host` field — the macro injects its own.
    let user_fields: Vec<&syn::Field> = match &item_struct.fields {
        syn::Fields::Unit => Vec::new(),
        syn::Fields::Named(named) => named
            .named
            .iter()
            .filter(|f| f.ident.as_ref().map(|i| i != "host").unwrap_or(true))
            .collect(),
        syn::Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                item_struct,
                "Storage struct must be a unit struct (`pub struct Foo;`) or have named fields.",
            ));
        }
    };

    let user_field_tokens: Vec<TokenStream> = user_fields.iter().map(|f| quote! { #f }).collect();

    Ok(quote! {
        #(#attrs)*
        #vis struct #name {
            /// Host handle. Use [`Self::host`] in contract code; tests may
            /// construct the struct directly with `Host::from_dyn(...)`.
            pub host: ::pvm_contract_sdk::Host,
            #(#user_field_tokens,)*
        }
    })
}

/// Whether this `impl` block targets the contract's storage struct, including
/// trait impls like `impl Trait for StorageStruct`.
fn impl_targets_storage_struct(item_impl: &syn::ItemImpl, struct_name: &Ident) -> bool {
    let syn::Type::Path(type_path) = item_impl.self_ty.as_ref() else {
        return false;
    };
    type_path
        .path
        .segments
        .last()
        .map(|s| &s.ident == struct_name)
        .unwrap_or(false)
}

/// Detect a struct with `#[derive(SolStorage)]` in the module items.
/// Returns the struct name if found, or an error if more than one non-cfg-gated
/// duplicate is found.
///
/// Proc macros see module items before `#[cfg]` evaluation, so feature-gated
/// storage structs (e.g. `#[cfg(feature = "v1")] struct StorageV1`) are visible
/// even when inactive. We allow multiple `SolStorage` structs as long as every
/// duplicate carries a `#[cfg(...)]` attribute, which indicates the developer
/// intends for exactly one to be active per build.
fn find_sol_storage_struct(input: &ItemMod) -> syn::Result<Option<Ident>> {
    let content = match input.content.as_ref() {
        Some(c) => c,
        None => return Ok(None),
    };

    // Collect all structs that derive SolStorage, noting whether they have #[cfg].
    let mut candidates: Vec<(&syn::ItemStruct, bool)> = Vec::new();
    for item in &content.1 {
        let syn::Item::Struct(s) = item else {
            continue;
        };
        if !has_sol_storage_derive(s) {
            continue;
        }
        let has_cfg = s.attrs.iter().any(|a| a.path().is_ident("cfg"));
        candidates.push((s, has_cfg));
    }

    if candidates.len() <= 1 {
        return Ok(candidates.first().map(|(s, _)| s.ident.clone()));
    }

    // Multiple candidates found.
    //
    // Proc macros run before #[cfg] evaluation, so feature-gated storage structs
    // are all visible even though only one will be active per build. We allow
    // this IF every candidate is #[cfg]-gated AND they all share the same name,
    // because the injected code references the struct by name, which must resolve
    // regardless of which cfg branch the compiler selects.
    let all_cfg_gated = candidates.iter().all(|(_, has_cfg)| *has_cfg);

    if all_cfg_gated {
        let first_name = &candidates[0].0.ident;
        for (s, _) in &candidates[1..] {
            if s.ident != *first_name {
                return Err(syn::Error::new_spanned(
                    s,
                    format!(
                        "cfg-gated #[derive(SolStorage)] structs must share the same name \
                         (found `{}` and `{}`); the #[contract] macro injects code that \
                         references the struct by name, which must resolve in every cfg branch",
                        first_name, s.ident
                    ),
                ));
            }
        }
        return Ok(Some(first_name.clone()));
    }

    // At least one candidate is unconditional. Reject the duplicate.
    let first_name = &candidates[0].0.ident;
    for (s, has_cfg) in &candidates[1..] {
        if !has_cfg {
            return Err(syn::Error::new_spanned(
                s,
                format!(
                    "only one #[derive(SolStorage)] struct is allowed per contract module \
                     (already found `{}`); if these are feature-gated, add #[cfg(...)] \
                     to each variant",
                    first_name
                ),
            ));
        }
    }
    // First candidate lacks cfg but later ones have it.
    Err(syn::Error::new_spanned(
        candidates[0].0,
        format!(
            "only one #[derive(SolStorage)] struct is allowed per contract module \
             (also found `{}`); if these are feature-gated, add #[cfg(...)] \
             to each variant",
            candidates[1].0.ident
        ),
    ))
}

/// Check whether a struct has `#[derive(SolStorage)]` by parsing the derive
/// token list and matching each path exactly (not substring).
fn has_sol_storage_derive(s: &syn::ItemStruct) -> bool {
    for attr in &s.attrs {
        if let syn::Meta::List(meta_list) = &attr.meta {
            if !meta_list.path.is_ident("derive") {
                continue;
            }
            // Parse the derive arguments as a comma-separated list of paths
            let paths: Result<syn::punctuated::Punctuated<syn::Path, syn::Token![,]>, _> =
                meta_list.parse_args_with(syn::punctuated::Punctuated::parse_terminated);
            if let Ok(paths) = paths {
                for path in &paths {
                    // Match both `SolStorage` (unqualified) and
                    // `pvm_contract_macros::SolStorage` (fully qualified).
                    // For multi-segment paths, verify the prefix is a known
                    // PVM macro crate name.
                    if path.is_ident("SolStorage") {
                        return true;
                    }
                    if path.segments.len() == 2 {
                        let prefix = path.segments[0].ident.to_string();
                        if VALID_PREFIXES.contains(&prefix.as_str())
                            && path.segments[1].ident == "SolStorage"
                        {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
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
    fn errors_when_module_has_no_struct() {
        let input: ItemMod = syn::parse_quote! {
            mod empty { }
        };
        let err = expand_contract(ContractArgs::default(), input).unwrap_err();
        assert!(err.to_string().contains("must contain a storage struct"));
    }

    #[test]
    fn errors_when_method_missing_self() {
        // A `#[method]` without a `self` receiver would expand into
        // `this.foo(args)` where `foo` is a free associated function — producing
        // a cryptic "no method named" error. Catch it at parse time instead.
        let input: ItemMod = syn::parse_quote! {
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::method]
                    pub fn foo(value: u32) -> u32 { value }
                }
            }
        };
        let err = expand_contract(ContractArgs::default(), input).unwrap_err();
        assert!(
            err.to_string().contains("&self"),
            "error should mention &self: {err}"
        );
    }

    #[test]
    fn errors_when_method_takes_owning_self() {
        // Owning `self` would consume the contract; dispatch must be able to
        // call multiple methods on the same instance, so only borrowing
        // receivers are allowed.
        let input: ItemMod = syn::parse_quote! {
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::method]
                    pub fn foo(self) -> u32 { 0 }
                }
            }
        };
        let err = expand_contract(ContractArgs::default(), input).unwrap_err();
        assert!(
            err.to_string().contains("borrowed self"),
            "error should mention borrowed self: {err}"
        );
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
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self, owner: Address, supply: U256) {}
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(output.contains("deploy"));
        assert!(output.contains("\"owner\""));
        assert!(output.contains("\"supply\""));
        assert!(output.contains("fn route"));
    }

    #[test]
    fn generates_router_impl_and_route_fn() {
        let item: syn::ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn balance_of(&self, account: Address) -> U256 {
                        U256::ZERO
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        // Generated route() takes a `&mut Contract` and returns Option<()>
        assert!(
            output.contains("fn route"),
            "route() function should be generated"
        );
        // The Router trait is instantiated at the concrete Host type
        assert!(
            output.contains("Router :: < :: pvm_contract_sdk :: Host >")
                || output.contains(":: pvm_contract_sdk :: Router"),
            "Router impl should target concrete Host"
        );
        // call() delegates to route() with the constructed `this` and falls
        // through to the unknown-selector handler when the Option is None.
        assert!(output.contains("route (& mut this , selector , input)"));
        assert!(output.contains("is_none ()"));
    }

    #[test]
    fn constructor_with_result_and_inputs_generates_match_and_decode() {
        let item: syn::ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self, owner: Address) -> Result<(), Error> {
                        Ok(())
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(output.contains("\"owner\""));
        assert!(output.contains("Err (e)"));
        assert!(output.contains("REVERT"));
    }

    #[test]
    fn user_impl_is_cfg_gated_for_abi_gen() {
        let item: syn::ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn do_something(&self, value: U256) -> U256 {
                        value
                    }
                }
            }
        "#,
        )
        .unwrap();

        let tokens = expand_contract(ContractArgs::default(), item).unwrap();
        let output = tokens.to_string();

        // User impl blocks are gated behind not(abi-gen) so method bodies
        // (which may call host APIs) are excluded from host-target ABI builds.
        assert!(
            output.contains("not (feature = \"abi-gen\")"),
            "user impl must be cfg-gated for abi-gen"
        );

        // The abi-gen helper still references the type for SOL_NAME
        assert!(
            output.contains("SOL_NAME"),
            "abi-gen helper must reference SOL_NAME"
        );
    }

    #[test]
    fn error_paths_do_not_emit_raw_bytes() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) -> Result<(), MyError> {
                        Ok(())
                    }

                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: u64) -> Result<(), MyError> {
                        Ok(())
                    }

                    #[pvm_contract_macros::fallback]
                    pub fn fallback(&mut self) -> Result<(), MyError> {
                        Ok(())
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

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
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::fallback]
                    pub fn fallback(&mut self) {}
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        // Unit-return fallback should generate a plain method call
        assert!(
            output.contains("this . fallback ()"),
            "Unit-return fallback should generate a direct method call on `this`"
        );
        assert!(
            !output.contains("match this . fallback"),
            "Unit-return fallback should not generate a match expression"
        );
    }

    #[test]
    fn contract_injects_storage_variable_when_sol_storage_present() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[derive(SolStorage)]
                struct Storage {
                    #[slot(0)]
                    counter: Lazy<U256>,
                }

                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn get_counter(&self) -> U256 {
                        U256::ZERO
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(
            output.contains("__pvm_storage"),
            "Contract should inject storage variable when SolStorage struct is present.\n\
             Expanded output:\n{output}"
        );

        assert!(
            output.contains("as :: pvm_contract_sdk :: SolStorage")
                && output.contains("__pvm_storage"),
            "Storage injection should use fully-qualified SolStorage::__pvm_storage().\n\
             Expanded output:\n{output}"
        );

        let pvm_storage_count = output.matches("__pvm_storage").count();
        assert!(
            pvm_storage_count >= 2,
            "Both constructor and method should get storage injection, \
             but found only {pvm_storage_count} occurrence(s).\n\
             Expanded output:\n{output}"
        );
    }

    #[test]
    fn contract_does_not_inject_storage_without_sol_storage() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn get_value(&self) -> U256 {
                        U256::ZERO
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(
            !output.contains("__pvm_storage"),
            "Contract should not inject storage when no SolStorage struct is present"
        );
    }

    #[test]
    fn contract_rejects_multiple_sol_storage_structs() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[derive(SolStorage)]
                struct StorageA {
                    #[slot(0)]
                    a: Lazy<U256>,
                }

                #[derive(SolStorage)]
                struct StorageB {
                    #[slot(1)]
                    b: Lazy<U256>,
                }

                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}
                }
            }
        "#,
        )
        .unwrap();

        let result = expand_contract(ContractArgs::default(), item);
        assert!(
            result.is_err(),
            "Should reject modules with multiple non-cfg-gated SolStorage structs"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only one #[derive(SolStorage)]"),
            "Error should mention the constraint. Got: {err}"
        );
    }

    #[test]
    fn contract_allows_cfg_gated_sol_storage_structs() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[cfg(feature = "v1")]
                #[derive(SolStorage)]
                struct Storage {
                    #[slot(0)]
                    a: Lazy<U256>,
                }

                #[cfg(not(feature = "v1"))]
                #[derive(SolStorage)]
                struct Storage {
                    #[slot(0)]
                    a: Lazy<U256>,
                    #[slot(1)]
                    b: Lazy<U256>,
                }

                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn get_value(&self) -> U256 {
                        U256::ZERO
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(
            output.contains("__pvm_storage"),
            "Contract should accept cfg-gated SolStorage structs and inject storage.\n\
             Expanded output:\n{output}"
        );
    }

    #[test]
    fn contract_rejects_cfg_gated_sol_storage_with_different_names() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[cfg(feature = "v1")]
                #[derive(SolStorage)]
                struct StorageV1 {
                    #[slot(0)]
                    a: Lazy<U256>,
                }

                #[cfg(not(feature = "v1"))]
                #[derive(SolStorage)]
                struct StorageV2 {
                    #[slot(0)]
                    a: Lazy<U256>,
                }

                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}
                }
            }
        "#,
        )
        .unwrap();

        let result = expand_contract(ContractArgs::default(), item);
        assert!(
            result.is_err(),
            "Should reject cfg-gated SolStorage structs with different names"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must share the same name"),
            "Error should explain the name requirement. Got: {err}"
        );
    }

    #[test]
    fn contract_does_not_match_sol_storage_substring() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[derive(NotSolStorage)]
                struct Storage {
                    value: u32,
                }

                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn get_value(&self) -> U256 {
                        U256::ZERO
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(
            !output.contains("__pvm_storage"),
            "Should not match derive names that merely contain 'SolStorage' as substring"
        );
    }

    #[test]
    fn contract_detects_fully_qualified_sol_storage_derive() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[derive(pvm_contract_macros::SolStorage)]
                struct Storage {
                    #[slot(0)]
                    counter: Lazy<U256>,
                }

                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn get_counter(&self) -> U256 {
                        U256::ZERO
                    }
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(
            output.contains("__pvm_storage"),
            "Contract should detect fully qualified pvm_contract_macros::SolStorage.\n\
             Expanded output:\n{output}"
        );
    }

    #[test]
    fn rejects_generic_contract_struct() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract<T>(::core::marker::PhantomData<T>);
                impl<T> MyContract<T> {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}
                }
            }
        "#,
        )
        .unwrap();

        let err = expand_contract(ContractArgs::default(), item)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("contract structs must not be generic"),
            "Expected struct-generic rejection. Got: {err}"
        );
    }

    #[test]
    fn rejects_generic_contract_impl() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract;
                impl<T> MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}
                }
            }
        "#,
        )
        .unwrap();

        let err = expand_contract(ContractArgs::default(), item)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("contract `impl` blocks must not be generic"),
            "Expected impl-generic rejection. Got: {err}"
        );
    }

    #[test]
    fn rejects_generic_contract_method() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn echo<T: Copy>(&self, x: T) -> T { x }
                }
            }
        "#,
        )
        .unwrap();

        let err = expand_contract(ContractArgs::default(), item)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("contract methods must not be generic"),
            "Expected method-generic rejection. Got: {err}"
        );
    }
}
