use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Ident, ItemMod, LitInt, LitStr, Token, parse::Parse, parse::ParseStream};
use syn_solidity::Item;

use super::abi_gen::generate_abi_gen;
use super::dispatch::{
    MethodInfo, RouteItems, StateMutability, boundary_size_check, generate_param_decoding,
    generate_revert_encoding_boundary, generate_router,
};
use super::sol_storage::extract_optional_slot_attr;
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
    /// True iff the constructor is marked `#[payable]`.
    pub(super) constructor_is_payable: bool,
    pub(super) fallback_name: Option<Ident>,
    pub(super) fallback_returns_result: bool,
    /// True iff the fallback is marked `#[payable]`.
    pub(super) fallback_is_payable: bool,
    pub(super) has_receive: bool,
    pub(super) receive_name: Option<Ident>,
    pub(super) receive_returns_result: bool,
    /// Error types from `Result<T, E>` return types, for ABI generation.
    pub(super) error_types: Vec<syn::Type>,
    /// Idents of structs in the module body carrying `#[derive(SolEvent)]`.
    /// Used by the abi-gen codepath to emit event entries for no-sol contracts.
    pub(super) event_idents: Vec<Ident>,
}

/// A storage field annotated with `#[slot(N)]` on the contract struct.
#[derive(Debug, Clone)]
pub(super) struct SlotField {
    pub name: Ident,
    pub ty: syn::Type,
    pub slot: u64,
    /// `#[cfg(...)]` attributes on the field, propagated into construction and layout.
    pub cfg_attrs: Vec<syn::Attribute>,
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

/// True iff `output` is `()`, an explicit unit tuple, or omitted entirely.
fn is_unit_return_type(output: &syn::ReturnType) -> bool {
    match output {
        syn::ReturnType::Default => true,
        syn::ReturnType::Type(_, ty) => matches!(
            ty.as_ref(),
            syn::Type::Tuple(t) if t.elems.is_empty()
        ),
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
/// Accepts methods with `&self`, `&mut self`, or no receiver (associated
/// functions, used for `pure` methods). Owned `self` is rejected — it would
/// consume the contract instance. The receiver, when present, is skipped;
/// remaining typed params are returned in order.
///
/// Mutability/payable enforcement is done by [`classify_receiver`] and
/// [`infer_method_mutability`] at the call site.
fn extract_typed_params_impl(
    _func: &syn::ImplItemFn,
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
) -> syn::Result<Vec<(Ident, syn::Type)>> {
    let skip = match inputs.first() {
        Some(syn::FnArg::Receiver(r)) if r.reference.is_none() => {
            return Err(syn::Error::new_spanned(
                r,
                "owning `self` would consume the contract instance; use `&self` or `&mut self`",
            ));
        }
        Some(syn::FnArg::Receiver(_)) => 1,
        _ => 0,
    };

    inputs
        .iter()
        .skip(skip)
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

/// Scan struct attributes for `#[derive(..., Clone, ...)]` (any path that
/// resolves to `Clone`, ignoring fully-qualified prefixes like
/// `core::clone::Clone`). Returns the offending derive token for span
/// reporting, or `None` if no `Clone` is derived.
fn find_derive_clone(attrs: &[Attribute]) -> Option<&Attribute> {
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let derives_clone = attr
            .parse_args_with(syn::punctuated::Punctuated::<syn::Path, Token![,]>::parse_terminated)
            .ok()
            .map(|paths| {
                paths
                    .iter()
                    .any(|p| p.segments.last().is_some_and(|s| s.ident == "Clone"))
            })
            .unwrap_or(false);
        if derives_clone {
            return Some(attr);
        }
    }
    None
}

/// `true` iff the function's first parameter is `&mut self`.
fn receiver_is_mut(inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>) -> bool {
    matches!(
        inputs.first(),
        Some(syn::FnArg::Receiver(r))
            if r.reference.is_some() && r.mutability.is_some()
    )
}

/// Method receiver classification used for mutability inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiverKind {
    /// No receiver — `fn foo(args) -> T`. Maps to Solidity `pure`.
    None,
    /// `&self` — read-only. Maps to Solidity `view`.
    Ref,
    /// `&mut self` — mutating. Maps to `nonpayable` / `payable`.
    RefMut,
}

fn classify_receiver(
    inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>,
) -> syn::Result<ReceiverKind> {
    match inputs.first() {
        None | Some(syn::FnArg::Typed(_)) => Ok(ReceiverKind::None),
        Some(syn::FnArg::Receiver(r)) => {
            if r.colon_token.is_some() {
                return Err(syn::Error::new_spanned(
                    r,
                    "explicit-type `self` receiver is not supported; use `&self` or `&mut self`",
                ));
            }
            if r.reference.is_none() {
                return Err(syn::Error::new_spanned(
                    r,
                    "consuming `self` receiver is not supported; use `&self` or `&mut self`",
                ));
            }
            if r.mutability.is_some() {
                Ok(ReceiverKind::RefMut)
            } else {
                Ok(ReceiverKind::Ref)
            }
        }
    }
}

/// Infer Solidity stateMutability from receiver + `#[payable]`.
///
/// | Receiver       | `#[payable]` | Result        |
/// |----------------|--------------|---------------|
/// | none           | no           | `Pure`        |
/// | none           | yes          | error         |
/// | `&self`        | no           | `View`        |
/// | `&self`        | yes          | error         |
/// | `&mut self`    | no           | `NonPayable`  |
/// | `&mut self`    | yes          | `Payable`     |
fn infer_method_mutability(
    func: &syn::ImplItemFn,
    is_payable: bool,
) -> syn::Result<StateMutability> {
    let kind = classify_receiver(&func.sig.inputs)?;
    match (kind, is_payable) {
        (ReceiverKind::None, false) => Ok(StateMutability::Pure),
        (ReceiverKind::None, true) => Err(syn::Error::new_spanned(
            func,
            "associated function (no `self` receiver) cannot be marked `#[payable]`; \
             payable callables must take `&mut self`",
        )),
        (ReceiverKind::Ref, false) => Ok(StateMutability::View),
        (ReceiverKind::Ref, true) => Err(syn::Error::new_spanned(
            func,
            "method is marked `#[payable]` but takes `&self`; \
             payable callables must take `&mut self`",
        )),
        (ReceiverKind::RefMut, false) => Ok(StateMutability::NonPayable),
        (ReceiverKind::RefMut, true) => Ok(StateMutability::Payable),
    }
}

/// Format a `.sol` vs Rust mutability mismatch into a human-readable error
/// pointing at the Rust method.
fn mutability_mismatch_error(
    func: &syn::ImplItemFn,
    fn_name: &str,
    sol: StateMutability,
    rust: StateMutability,
) -> syn::Error {
    let hint = match (sol, rust) {
        (StateMutability::View, StateMutability::NonPayable) => "change Rust receiver to `&self`",
        (StateMutability::View, StateMutability::Payable) => {
            "remove `#[payable]` and change to `&self`"
        }
        (StateMutability::View, StateMutability::Pure) => "change Rust signature to take `&self`",
        (StateMutability::Pure, StateMutability::View) => {
            "remove `&self` (associated functions are pure)"
        }
        (StateMutability::Pure, StateMutability::NonPayable) => {
            "remove `&mut self` (associated functions are pure)"
        }
        (StateMutability::Pure, StateMutability::Payable) => {
            "remove `&mut self` and `#[payable]` (associated functions are pure)"
        }
        (StateMutability::NonPayable, StateMutability::View) => {
            "change Rust receiver to `&mut self`"
        }
        (StateMutability::NonPayable, StateMutability::Pure) => "add a `&mut self` receiver",
        (StateMutability::NonPayable, StateMutability::Payable) => "remove `#[payable]`",
        (StateMutability::Payable, StateMutability::NonPayable) => "add `#[payable]`",
        (StateMutability::Payable, StateMutability::View) => {
            "change to `&mut self` and add `#[payable]`"
        }
        (StateMutability::Payable, StateMutability::Pure) => {
            "add a `&mut self` receiver and `#[payable]`"
        }
        _ => "update either the `.sol` interface or the Rust signature",
    };
    syn::Error::new_spanned(
        func,
        format!(
            "method `{fn_name}` mutability mismatch: `.sol` declares `{}`, \
             Rust signature is `{}`. {}.",
            sol.as_abi_str(),
            rust.as_abi_str(),
            hint,
        ),
    )
}

/// Shared payable helpers emitted once per contract module so call sites
/// collapse to a single function call. `__pvm_assert_value_zero` reverts on a
/// boolean flag so mixed-payability dispatchers can read `value_transferred`
/// once into `__has_value` and have each non-payable arm tail-call the assert.
/// `__pvm_assert_non_payable` is the read+assert combinator used by the
/// deploy / fallback boundaries and by the all-non-payable router prelude;
/// the read itself goes through
/// `pvm_contract_sdk::value_transferred_is_nonzero`, which folds the 32-byte
/// buffer with a 4-word OR on riscv64 (smaller bytecode than `memcmp`).
fn build_payable_helpers_fn() -> TokenStream {
    quote! {
        #[cfg(not(feature = "abi-gen"))]
        #[inline(never)]
        fn __pvm_assert_value_zero(host: &::pvm_contract_sdk::Host, has_value: bool) {
            if has_value {
                <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                    host,
                    ::pvm_contract_sdk::ReturnFlags::REVERT,
                    &::pvm_contract_sdk::framework_errors::NON_PAYABLE_VALUE_RECEIVED);
            }
        }

        #[cfg(not(feature = "abi-gen"))]
        #[inline(never)]
        fn __pvm_assert_non_payable(host: &::pvm_contract_sdk::Host) {
            __pvm_assert_value_zero(
                host,
                ::pvm_contract_sdk::value_transferred_is_nonzero(host),
            );
        }
    }
}

/// Emit a call to the shared non-payable assertion helper. Used by deploy /
/// fallback boundaries (which already have `this` in scope).
fn build_assert_non_payable_call(emit: bool) -> TokenStream {
    if !emit {
        return quote! {};
    }
    quote! { __pvm_assert_non_payable(this.host()); }
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
                    || has_pvm_attr(&f.attrs, "receive")
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
            {
                if !item_struct.generics.params.is_empty() {
                    return Err(syn::Error::new_spanned(
                        &item_struct.generics.params,
                        "contract structs must not be generic",
                    ));
                }
                // The contract storage struct is the borrow-check root for
                // mutation gating: a `&self` method holds `&Storage`, a
                // `&mut self` method holds `&mut Storage`. If the user
                // derives `Clone`, a view method could clone the storage and
                // get a fresh `&mut Storage` — bypassing the gate and lying
                // to the ABI. Reject `#[derive(Clone)]` syntactically.
                if let Some(bad) = find_derive_clone(&item_struct.attrs) {
                    return Err(syn::Error::new_spanned(
                        bad,
                        "contract storage structs must not derive `Clone`; the \
                         mutation gate (`&self` vs `&mut self`) relies on \
                         `Storage: !Clone` to prevent view methods from \
                         smuggling out a `&mut Storage`",
                    ));
                }
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
                    || has_pvm_attr(&func.attrs, "fallback")
                    || has_pvm_attr(&func.attrs, "receive"))
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
    let mut constructor_is_payable = false;
    let mut fallback_name = None;
    let mut fallback_returns_result = false;
    let mut fallback_is_payable = false;
    let mut has_receive = false;
    let mut receive_name = None;
    let mut receive_returns_result = false;
    let mut implemented_sol_methods = Vec::new();
    let mut error_types: Vec<syn::Type> = Vec::new();
    let mut seen_error_names: Vec<String> = Vec::new();
    let mut event_idents: Vec<Ident> = Vec::new();

    for item in &content.1 {
        // Collect event structs with #[derive(SolEvent)]
        if let syn::Item::Struct(item_struct) = item
            && has_sol_event_derive(&item_struct.attrs)
        {
            event_idents.push(item_struct.ident.clone());
        }

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
                constructor_is_payable = has_pvm_attr(&func.attrs, "payable");
                // Constructors must take `&mut self`. A view (`&self`) or pure
                // (no receiver) constructor cannot initialize storage, so it
                // would never be a useful entry point. Reject explicitly.
                if !receiver_is_mut(&func.sig.inputs) {
                    return Err(syn::Error::new_spanned(
                        func,
                        "constructor must take `&mut self`; it always initializes storage",
                    ));
                }
                constructor_inputs = extract_typed_params_impl(func, &func.sig.inputs)?;
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
            } else if has_pvm_attr(&func.attrs, "fallback") {
                has_fallback = true;
                fallback_name = Some(func.sig.ident.clone());
                fallback_returns_result = is_result_return_type(&func.sig.output);
                fallback_is_payable = has_pvm_attr(&func.attrs, "payable");
                // Fallback dispatch generates `this.fallback_name()` (method
                // call), so the fallback must have a receiver. `&self` and
                // `&mut self` are both valid; no-receiver (pure) fallback is
                // rejected here — a pure fallback has no host access and is
                // never useful (can't read calldata, return values, or state).
                match classify_receiver(&func.sig.inputs)? {
                    ReceiverKind::Ref | ReceiverKind::RefMut => {}
                    ReceiverKind::None => {
                        return Err(syn::Error::new_spanned(
                            func,
                            "fallback must take `&self` or `&mut self`; \
                             a no-receiver fallback has no access to host or calldata",
                        ));
                    }
                }
                // Reuses the payable+receiver consistency check.
                let _ = infer_method_mutability(func, fallback_is_payable)?;
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
            } else if has_pvm_attr(&func.attrs, "receive") {
                if has_receive {
                    return Err(syn::Error::new_spanned(
                        func,
                        "duplicate `#[receive]` handler; a contract may declare at most one",
                    ));
                }
                if has_pvm_attr(&func.attrs, "payable") {
                    return Err(syn::Error::new_spanned(
                        func,
                        "`#[receive]` is implicitly payable; remove the redundant `#[payable]` attribute",
                    ));
                }
                if !receiver_is_mut(&func.sig.inputs) {
                    return Err(syn::Error::new_spanned(
                        func,
                        "`#[receive]` must take `&mut self`",
                    ));
                }
                if func.sig.inputs.len() != 1 {
                    return Err(syn::Error::new_spanned(
                        func,
                        "`#[receive]` must take no arguments other than `&mut self`",
                    ));
                }
                receive_returns_result = is_result_return_type(&func.sig.output);
                if !receive_returns_result && !is_unit_return_type(&func.sig.output) {
                    return Err(syn::Error::new_spanned(
                        &func.sig.output,
                        "`#[receive]` must return `()` or `Result<(), E>`; Solidity's receive cannot return a value",
                    ));
                }
                has_receive = true;
                receive_name = Some(func.sig.ident.clone());
                collect_error_type(&func.sig.output, &mut error_types, &mut seen_error_names);
            } else if has_pvm_attr(&func.attrs, "method") {
                let typed_params = extract_typed_params_impl(func, &func.sig.inputs)?;
                let is_payable = has_pvm_attr(&func.attrs, "payable");
                let inferred_mutability = infer_method_mutability(func, is_payable)?;
                let param_names: Vec<Ident> = typed_params.iter().map(|(n, _)| n.clone()).collect();
                let param_types: Vec<syn::Type> =
                    typed_params.into_iter().map(|(_, t)| t).collect();

                let returns_result = is_result_return_type(&func.sig.output);
                let return_types = extract_return_types(&func.sig.output);

                let (sol_name, precomputed_selector, mutability) = if let Some(sol_iface) =
                    sol_interface
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
                    let sol_mutability = match sol_func.attributes.mutability() {
                        Some(syn_solidity::Mutability::Pure(_)) => StateMutability::Pure,
                        Some(syn_solidity::Mutability::View(_)) => StateMutability::View,
                        Some(syn_solidity::Mutability::Payable(_)) => StateMutability::Payable,
                        _ => StateMutability::NonPayable,
                    };
                    if sol_mutability != inferred_mutability {
                        return Err(mutability_mismatch_error(
                            func,
                            &func.sig.ident.to_string(),
                            sol_mutability,
                            inferred_mutability,
                        ));
                    }
                    (
                        sol_func.name().to_string(),
                        Some(selector),
                        inferred_mutability,
                    )
                } else {
                    let sol_name = extract_method_rename(&func.attrs)?
                        .unwrap_or_else(|| to_camel_case(&func.sig.ident.to_string()));
                    (sol_name, None, inferred_mutability)
                };

                methods.push(MethodInfo {
                    fn_name: func.sig.ident.clone(),
                    sol_name,
                    param_names,
                    param_types,
                    return_types,
                    returns_result,
                    mutability,
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
                syn_solidity::Item::Function(item_function)
                    if matches!(item_function.kind, syn_solidity::FunctionKind::Function(_)) =>
                {
                    Some(item_function)
                }
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

        // Validate every Rust `#[derive(SolEvent)]` struct has a matching
        // `event Name(...)` declaration in the `.sol` interface. Without this
        // check, a Rust event declared without a corresponding `.sol` entry
        // would be silently absent from the generated ABI JSON (the builder
        // reads events from `.sol` when a sol_path is set).
        let sol_event_names: Vec<String> = sol_iface
            .body
            .iter()
            .filter_map(|item| match item {
                syn_solidity::Item::Event(item_event) => Some(item_event.name.to_string()),
                _ => None,
            })
            .collect();
        let missing_events: Vec<String> = event_idents
            .iter()
            .map(|ident| ident.to_string())
            .filter(|name| !sol_event_names.contains(name))
            .collect();
        if !missing_events.is_empty() {
            return Err(syn::Error::new_spanned(
                input,
                format!(
                    "Rust events missing matching `event` declarations in the .sol interface: {}",
                    missing_events.join(", ")
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
        constructor_is_payable,
        fallback_name,
        fallback_returns_result,
        fallback_is_payable,
        has_receive,
        receive_name,
        receive_returns_result,
        error_types,
        event_idents,
    })
}

/// match both `SolEvent` and paths ending in `SolEvent` (e.g.
/// `pvm_contract_macros::SolEvent`).
fn has_sol_event_derive(attrs: &[Attribute]) -> bool {
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(last) = meta.path.segments.last()
                && last.ident == "SolEvent"
            {
                found = true;
            }
            Ok(())
        });
        if found {
            return true;
        }
    }
    false
}

pub fn expand_contract(args: ContractArgs, input: ItemMod) -> syn::Result<TokenStream> {
    let sol_interface = if let Some(ref path) = args.sol_path {
        Some(load_sol_interface(path).map_err(|e| syn::Error::new_spanned(&input, e))?)
    } else {
        None
    };

    let parsed = parse_contract(&input, sol_interface.as_ref())?;
    let use_alloc = args.allocator.is_some();

    let mod_name = &parsed.mod_name;
    let mod_vis = &input.vis;
    let mod_attrs = &input.attrs;

    let struct_name = parsed.struct_name.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(
            &input,
            "Contract module must contain a storage struct (e.g. `pub struct Foo;`)",
        )
    })?;

    let slot_fields = extract_slot_fields(&input, struct_name)?;
    let (abi_gen_helper, abi_gen_main) =
        generate_abi_gen(&parsed, args.sol_path.is_some(), &slot_fields);

    let mod_content = strip_pvm_attrs(&input, struct_name)?;

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

    // Generate the `this` construction, shared by deploy() and call().
    // When #[slot(N)] fields are present, each is initialised with a clone
    // of the host handle so storage cells share the same backing store.
    let slot_field_inits: Vec<TokenStream> = slot_fields
        .iter()
        .map(|sf| {
            let name = &sf.name;
            let ty = &sf.ty;
            let slot = sf.slot;
            let cfgs = &sf.cfg_attrs;
            quote! {
                #(#cfgs)*
                #name: <#ty>::new(
                    ::pvm_contract_sdk::StorageKey::from_slot(#slot),
                    host.clone(),
                )
            }
        })
        .collect();
    let this_construction = quote! {
        let host = ::pvm_contract_sdk::Host::new();
        let mut this = #struct_name {
            #(#slot_field_inits,)*
            host,
        };
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

        let decoding = generate_param_decoding(&param_names, &param_types, true);
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

        let deploy_assert = build_assert_non_payable_call(!parsed.constructor_is_payable);

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
                #this_construction
                #deploy_assert
                #read_calldata
                #decode_and_call
            }
        }
    } else {
        // No user-declared constructor — emit a default payable-guarded deploy so
        // deployments with value revert, matching Solidity's default behaviour.
        quote! {
            #[cfg(target_arch = "riscv64")]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn deploy() {
                use ::pvm_contract_sdk::pallet_revive_uapi::HostFn as _;
                #this_construction
                __pvm_assert_non_payable(this.host());
            }
        }
    };

    let (route_items, router_impl) =
        generate_router(&parsed.methods, mod_name, struct_name, use_alloc);
    let RouteItems { route_fn } = route_items;
    let router_impl = router_impl.tokens;

    // When `#[receive]` is present, the empty-calldata case dispatches to it
    // before falling through to the no-selector path. Receive is implicitly
    // payable, so no value guard is emitted.
    let receive_dispatch = if parsed.has_receive {
        let receive_name = parsed.receive_name.as_ref().unwrap();
        if parsed.receive_returns_result {
            let revert_err = generate_revert_encoding_boundary(use_alloc);
            quote! {
                if call_data_len == 0 {
                    match this.#receive_name() {
                        Ok(()) => return,
                        Err(e) => { #revert_err }
                    }
                }
            }
        } else {
            quote! {
                if call_data_len == 0 {
                    this.#receive_name();
                    return;
                }
            }
        }
    } else {
        quote! {}
    };

    let (no_selector_handler, unknown_selector_handler) = if parsed.has_fallback {
        let fallback_name = parsed.fallback_name.as_ref().unwrap();

        let fallback_assert = build_assert_non_payable_call(!parsed.fallback_is_payable);

        let handler = if parsed.fallback_returns_result {
            let revert_err = generate_revert_encoding_boundary(use_alloc);
            quote! {
                #fallback_assert
                match this.#fallback_name() {
                    Ok(()) => return,
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        } else {
            quote! {
                #fallback_assert
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
                #this_construction
                let call_data_len = ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = alloc::vec![0u8; call_data_len];
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data, 0);

                if call_data_len < 4 {
                    #receive_dispatch
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
                #this_construction
                let call_data_len = ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_size() as usize;
                let mut call_data = [0u8; #buffer_size];
                if call_data_len > #buffer_size {
                    ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                        ::pvm_contract_sdk::ReturnFlags::REVERT,
                        &::pvm_contract_sdk::framework_errors::CALLDATA_TOO_LARGE);
                }
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_copy(&mut call_data[..call_data_len], 0);

                if call_data_len < 4 {
                    #receive_dispatch
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

    let payable_helpers_fn = build_payable_helpers_fn();

    Ok(quote! {
        #alloc_setup

        #panic_handler

        #(#mod_attrs)*
        #mod_vis mod #mod_name {
            #mod_content

            #payable_helpers_fn

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
/// - Strip `#[slot(N)]` attrs from struct fields.
/// - Emit an `impl StorageStruct { fn host(&self) -> &Host }` accessor.
///
/// All user `impl` blocks are cfg-gated to `not(feature = "abi-gen")` so their
/// bodies (which may call host APIs) are excluded from host-target ABI builds.
fn strip_pvm_attrs(input: &ItemMod, struct_name: &Ident) -> syn::Result<TokenStream> {
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
                for impl_item in new_impl.items.iter_mut() {
                    if let syn::ImplItem::Fn(func) = impl_item {
                        func.attrs.retain(|attr| {
                            let segments: Vec<_> = attr.path().segments.iter().collect();
                            !(segments.len() == 2
                                && VALID_PREFIXES.contains(&segments[0].ident.to_string().as_str())
                                && (segments[1].ident == "method"
                                    || segments[1].ident == "constructor"
                                    || segments[1].ident == "fallback"
                                    || segments[1].ident == "receive"))
                        });
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
    //
    // Also auto-implement `ContractContext` (and its sealing trait) on the
    // contract storage struct. Cross-contract call builders take
    // `&impl ContractContext` (view/pure) or `&mut impl ContractContext`
    // (nonpayable/payable), so the borrow on `Self` is the gate that prevents
    // a `&self` method from initiating a state-mutating call.
    let host_accessor = quote! {
        #[cfg(not(feature = "abi-gen"))]
        impl #struct_name {
            #[inline(always)]
            pub fn host(&self) -> &::pvm_contract_sdk::Host {
                &self.host
            }
        }

        #[cfg(not(feature = "abi-gen"))]
        impl ::pvm_contract_sdk::__private::Sealed for #struct_name {}

        #[cfg(not(feature = "abi-gen"))]
        impl ::pvm_contract_sdk::ContractContext for #struct_name {
            #[inline(always)]
            fn host(&self) -> &::pvm_contract_sdk::Host {
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

/// Rewrite a user-declared storage struct into `pub struct Name { host: Host, <user fields> }`.
/// Accepts unit (`pub struct Name;`) or named (`pub struct Name { ... }`) forms.
/// Strips `#[slot(N)]` attributes from fields. They are consumed by
/// [`extract_slot_fields`] for construction and ABI generation.
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

    // Emit each user field but strip `#[slot(N)]` attributes.
    let user_field_tokens: Vec<TokenStream> = user_fields
        .iter()
        .map(|f| {
            let field_attrs: Vec<_> = f
                .attrs
                .iter()
                .filter(|a| !a.path().is_ident("slot"))
                .collect();
            let vis = &f.vis;
            let ident = &f.ident;
            let ty = &f.ty;
            quote! { #(#field_attrs)* #vis #ident: #ty }
        })
        .collect();

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

/// Extract `#[slot(N)]` fields from the contract struct.
///
/// Returns an empty vec for unit structs or structs with no `#[slot]` fields.
/// Validates there are no duplicate slot numbers.
fn extract_slot_fields(input: &ItemMod, struct_name: &Ident) -> syn::Result<Vec<SlotField>> {
    let content = input.content.as_ref().unwrap();
    for item in &content.1 {
        if let syn::Item::Struct(item_struct) = item
            && &item_struct.ident == struct_name
        {
            return extract_slot_fields_from_struct(item_struct);
        }
    }
    Ok(vec![])
}

fn extract_slot_fields_from_struct(item_struct: &syn::ItemStruct) -> syn::Result<Vec<SlotField>> {
    let named = match &item_struct.fields {
        syn::Fields::Unit => return Ok(vec![]),
        syn::Fields::Named(named) => named,
        syn::Fields::Unnamed(_) => return Ok(vec![]),
    };

    let mut fields = Vec::new();
    for field in &named.named {
        let Some(ident) = &field.ident else {
            continue;
        };
        if ident == "host" {
            if extract_optional_slot_attr(field)?.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "`host` is a reserved field name injected by the #[contract] macro. \
                     Rename this storage field.",
                ));
            }
            continue;
        }
        let Some(slot) = extract_optional_slot_attr(field)? else {
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "field `{ident}` must have a `#[slot(N)]` attribute. \
                     All non-host fields on the contract struct are storage fields \
                     and require a slot number."
                ),
            ));
        };
        let cfg_attrs: Vec<syn::Attribute> = field
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("cfg"))
            .cloned()
            .collect();
        fields.push(SlotField {
            name: ident.clone(),
            ty: field.ty.clone(),
            slot,
            cfg_attrs,
        });
    }

    // Reject duplicate slot numbers. When both fields are #[cfg]-gated
    // AND share the same name, we allow it. The compiler enforces that
    // only one field with a given name exists, so exactly one cfg branch
    // will be active. Different names with the same slot are always
    // rejected because the compiler can't catch the aliasing.
    for (i, a) in fields.iter().enumerate() {
        for b in &fields[i + 1..] {
            if a.slot != b.slot {
                continue;
            }
            let both_cfg = !a.cfg_attrs.is_empty() && !b.cfg_attrs.is_empty();
            let same_name = a.name == b.name;
            if both_cfg && same_name {
                continue;
            }
            return Err(syn::Error::new_spanned(
                item_struct,
                format!(
                    "duplicate slot {}: fields `{}` and `{}` use the same slot number",
                    a.slot, a.name, b.name
                ),
            ));
        }
    }

    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::super::dispatch::StateMutability;
    use super::{ContractArgs, expand_contract, parse_contract};
    use proc_macro2::TokenStream;
    use syn::ItemMod;

    fn parse_sol(src: &str) -> syn_solidity::File {
        let ts: proc_macro2::TokenStream = syn::parse_str(src).expect("solidity source parses");
        syn_solidity::parse2(ts).expect("syn_solidity parses")
    }

    #[test]
    fn parse_contract_detects_payable_attribute() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit(&mut self, to: Address) {}

                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: Address, amount: U256) -> bool { false }
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        let deposit = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "deposit")
            .unwrap();
        let transfer = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "transfer")
            .unwrap();
        assert_eq!(deposit.mutability, StateMutability::Payable);
        assert_eq!(transfer.mutability, StateMutability::NonPayable);
    }

    #[test]
    fn parse_contract_payable_attribute_keeps_all_params() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit(&mut self, to: Address) {}
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        let deposit = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "deposit")
            .unwrap();
        assert_eq!(deposit.sol_name, "deposit");
        assert_eq!(deposit.param_names.len(), 1);
        assert_eq!(deposit.param_names[0].to_string(), "to");
        assert_eq!(deposit.param_types.len(), 1);
    }

    #[test]
    fn parse_contract_payable_constructor() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    #[pvm_contract_macros::payable]
                    pub fn new(&mut self, initial: U256) {}
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        assert!(parsed.constructor_is_payable);
        assert_eq!(parsed.constructor_inputs.len(), 1);
    }

    #[test]
    fn parse_contract_non_payable_constructor() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self, initial: U256) {}
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        assert!(!parsed.constructor_is_payable);
    }

    #[test]
    fn parse_contract_payable_fallback() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::fallback]
                    #[pvm_contract_macros::payable]
                    pub fn any(&mut self) {}
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        assert!(parsed.fallback_is_payable);
    }

    #[test]
    fn parse_contract_view_from_sol() {
        let iface = parse_sol(
            r#"
            interface I {
                function balance() external view returns (uint256);
            }
        "#,
        );
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn balance(&self) -> U256 { U256::ZERO }
                }
            }
        };
        let parsed = parse_contract(&input, Some(&iface)).unwrap();
        let method = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "balance")
            .unwrap();
        assert_eq!(method.mutability, StateMutability::View);
    }

    #[test]
    fn parse_contract_pure_from_sol() {
        let iface = parse_sol(
            r#"
            interface I {
                function add(uint256 a, uint256 b) external pure returns (uint256);
            }
        "#,
        );
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn add(a: U256, b: U256) -> U256 { U256::ZERO }
                }
            }
        };
        let parsed = parse_contract(&input, Some(&iface)).unwrap();
        let method = parsed.methods.iter().find(|m| m.fn_name == "add").unwrap();
        assert_eq!(method.mutability, StateMutability::Pure);
    }

    #[test]
    fn parse_contract_pure_with_self_rejected() {
        // `.sol` declares `pure`, but Rust takes `&self` — pure functions
        // cannot have host access, so the receiver must be absent.
        let iface = parse_sol(
            r#"
            interface I {
                function add(uint256 a, uint256 b) external pure returns (uint256);
            }
        "#,
        );
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn add(&self, a: U256, b: U256) -> U256 { U256::ZERO }
                }
            }
        };
        let err = match parse_contract(&input, Some(&iface)) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("mutability mismatch") && msg.contains("pure") && msg.contains("view"),
            "expected pure/view mismatch, got: {msg}"
        );
    }

    #[test]
    fn parse_contract_view_mismatch_with_mut_self_rejected() {
        let iface = parse_sol(
            r#"
            interface I {
                function balance() external view returns (uint256);
            }
        "#,
        );
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn balance(&mut self) -> U256 { U256::ZERO }
                }
            }
        };
        let err = match parse_contract(&input, Some(&iface)) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("mutability mismatch")
                && msg.contains("view")
                && msg.contains("nonpayable"),
            "expected view/nonpayable mismatch, got: {msg}"
        );
    }

    #[test]
    fn parse_contract_nonpayable_from_sol_leaves_flags_false() {
        let iface = parse_sol(
            r#"
            interface I {
                function transfer(address to, uint256 amount) external returns (bool);
            }
        "#,
        );
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: Address, amount: U256) -> bool { false }
                }
            }
        };
        let parsed = parse_contract(&input, Some(&iface)).unwrap();
        let method = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "transfer")
            .unwrap();
        assert_eq!(method.mutability, StateMutability::NonPayable);
    }

    #[test]
    fn parse_contract_without_sol_infers_view_from_ref_self() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn balance(&self) -> U256 { U256::ZERO }
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        let method = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "balance")
            .unwrap();
        assert_eq!(method.mutability, StateMutability::View);
    }

    #[test]
    fn parse_contract_without_sol_infers_pure_from_no_receiver() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn version() -> u32 { 1 }
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        let method = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "version")
            .unwrap();
        assert_eq!(method.mutability, StateMutability::Pure);
    }

    #[test]
    fn parse_contract_without_sol_infers_nonpayable_from_mut_self() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self) {}
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        let method = parsed
            .methods
            .iter()
            .find(|m| m.fn_name == "transfer")
            .unwrap();
        assert_eq!(method.mutability, StateMutability::NonPayable);
    }

    #[test]
    fn parse_contract_payable_on_ref_self_rejected() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit(&self) {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.to_string().contains("payable") && err.to_string().contains("&self"),
            "expected payable+&self error, got: {}",
            err
        );
    }

    #[test]
    fn parse_contract_rejects_clone_on_storage_struct() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                #[derive(Clone)]
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn balance(&self) -> U256 { U256::ZERO }
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.to_string().contains("must not derive `Clone`"),
            "expected Clone rejection, got: {err}"
        );
    }

    #[test]
    fn parse_contract_rejects_clone_in_multi_derive() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                #[derive(Debug, Clone, PartialEq)]
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn balance(&self) -> U256 { U256::ZERO }
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.to_string().contains("must not derive `Clone`"),
            "expected Clone rejection in multi-derive, got: {err}"
        );
    }

    #[test]
    fn parse_contract_constructor_with_ref_self_rejected() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&self) {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.to_string()
                .contains("constructor must take `&mut self`"),
            "expected constructor mutability rejection, got: {err}"
        );
    }

    #[test]
    fn parse_contract_fallback_with_no_receiver_rejected() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::fallback]
                    pub fn fb() {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.to_string().contains("fallback must take")
                || err.to_string().contains("no-receiver"),
            "expected fallback receiver rejection, got: {err}"
        );
    }

    #[test]
    fn parse_contract_constructor_with_no_receiver_rejected() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new() {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.to_string()
                .contains("constructor must take `&mut self`"),
            "expected constructor mutability rejection, got: {err}"
        );
    }

    #[test]
    fn parse_contract_payable_on_no_receiver_rejected() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit() {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Err(e) => e,
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.to_string().contains("payable"),
            "expected payable error on no-receiver method, got: {}",
            err
        );
    }

    #[test]
    fn parse_contract_non_payable_fallback() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::fallback]
                    pub fn any(&mut self) {}
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        assert!(!parsed.fallback_is_payable);
    }

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
    fn method_without_receiver_is_pure() {
        // No `self` receiver = associated function = `pure` mutability.
        // Dispatch generates `MyContract::foo(args)` (UFCS) instead of
        // `this.foo(args)` so the call type-checks.
        let input: ItemMod = syn::parse_quote! {
            mod my_contract {
                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::method]
                    pub fn foo(value: u32) -> u32 { value }
                }
            }
        };
        let _ts = expand_contract(ContractArgs::default(), input)
            .expect("no-receiver method should be accepted as `pure`");
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
        let msg = err.to_string();
        assert!(
            msg.contains("consume the contract") || msg.contains("&self"),
            "error should reject owning self, got: {msg}"
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
        // The Router trait impl is emitted (no generic parameter).
        assert!(
            output.contains(":: pvm_contract_sdk :: Router"),
            "Router impl should be generated"
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
    fn call_body_always_emits_value_guard_for_non_payable_methods() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: Address) -> bool { false }
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        assert!(
            s.contains("__pvm_assert_non_payable"),
            "non-payable contract must emit the shared value-assertion helper"
        );
        assert!(
            s.contains("value_transferred"),
            "non-payable helper must call value_transferred to enforce rejection"
        );
        assert!(
            s.contains("NON_PAYABLE_VALUE_RECEIVED"),
            "non-payable helper must revert with NON_PAYABLE_VALUE_RECEIVED when value attached"
        );
    }

    #[test]
    fn event_structs_inside_module_are_wired_into_abi_gen() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[derive(pvm_contract_macros::SolEvent)]
                struct Transfer {
                    #[indexed] from: Address,
                    #[indexed] to: Address,
                    value: U256,
                }

                pub struct MyContract;
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: Address, amount: U256) {}
                }
            }
        "#,
        )
        .unwrap();

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(
            output.contains("Transfer :: abi_item") || output.contains("Transfer::abi_item"),
            "abi-gen output should reference Transfer::abi_item(), got: {output}"
        );
    }

    #[test]
    fn call_body_omits_value_code_when_all_payable() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit(&mut self) {}
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        let route_start = s.find("fn route").unwrap();
        let route_after = &s[route_start..];
        let route_end = route_after[4..]
            .find("fn ")
            .map(|i| i + 4)
            .unwrap_or(route_after.len());
        let route_body = &route_after[..route_end];
        assert!(
            !route_body.contains("__has_value"),
            "all-payable route must not emit __has_value; got:\n{route_body}"
        );
        assert!(
            !route_body.contains("__pvm_assert_non_payable"),
            "all-payable route must not invoke the non-payable helper; got:\n{route_body}"
        );
    }

    #[test]
    fn mixed_contract_emits_guard_for_non_payable_arms_only() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: Address) -> bool { false }
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        assert!(s.contains("__has_value"), "hoist missing: {s}");
        assert!(
            s.contains("__pvm_assert_value_zero"),
            "per-arm assert call missing: {s}"
        );
        assert!(
            s.contains("NON_PAYABLE_VALUE_RECEIVED"),
            "non-payable guard missing: {s}"
        );
    }

    #[test]
    fn deploy_non_payable_constructor_always_has_guard() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self, initial: U256) {}
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        assert!(
            s.contains("fn deploy"),
            "contract should emit deploy entry point"
        );
        let after_deploy = &s[s.find("fn deploy").unwrap()..];
        let deploy_end = after_deploy[4..]
            .find("fn ")
            .map(|i| i + 4)
            .unwrap_or(after_deploy.len());
        let deploy_body = &after_deploy[..deploy_end];
        assert!(
            deploy_body.contains("__pvm_assert_non_payable"),
            "non-payable constructor must invoke the shared value-assertion helper; got:\n{deploy_body}"
        );
    }

    #[test]
    fn deploy_payable_constructor_omits_guard() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    #[pvm_contract_macros::payable]
                    pub fn new(&mut self, initial: U256) {}
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        let after_deploy = &s[s.find("fn deploy").unwrap()..];
        let deploy_end = after_deploy[4..]
            .find("fn ")
            .map(|i| i + 4)
            .unwrap_or(after_deploy.len());
        let deploy_body = &after_deploy[..deploy_end];
        assert!(
            !deploy_body.contains("__pvm_assert_non_payable"),
            "payable constructor must not invoke the non-payable helper; got:\n{deploy_body}"
        );
    }

    #[test]
    fn fallback_non_payable_always_has_guard() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::fallback]
                    pub fn any(&mut self) {}
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        assert!(
            s.contains("NON_PAYABLE_VALUE_RECEIVED"),
            "non-payable fallback must always emit guard; got:\n{s}"
        );
    }

    #[test]
    fn fallback_payable_omits_guard() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::fallback]
                    #[pvm_contract_macros::payable]
                    pub fn any(&mut self) {}
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        let call_start = s.find("fn call").unwrap();
        let call_after = &s[call_start..];
        let call_end = call_after[4..]
            .find("fn ")
            .map(|i| i + 4)
            .unwrap_or(call_after.len());
        let call_body = &call_after[..call_end];
        assert!(
            !call_body.contains("__pvm_assert_non_payable"),
            "payable fallback must not invoke the non-payable helper in call(); got:\n{call_body}"
        );
    }

    #[test]
    fn contract_without_msg_value_still_guards_value() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn get(&self) -> U256 { U256::ZERO }
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        assert!(
            s.contains("__pvm_assert_non_payable"),
            "non-payable contract must invoke the shared value-assertion helper"
        );
        assert!(
            s.contains("value_transferred"),
            "non-payable contract must call value_transferred through the helper"
        );
        assert!(
            s.contains("NON_PAYABLE_VALUE_RECEIVED"),
            "non-payable contract must revert with NON_PAYABLE_VALUE_RECEIVED when value attached"
        );
    }

    #[test]
    fn mixed_contract_guards_non_payable_methods() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    #[pvm_contract_macros::payable]
                    pub fn deposit(&mut self) {}

                    #[pvm_contract_macros::method]
                    pub fn transfer(&mut self, to: Address) -> bool { false }
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = tokens.to_string();
        assert!(
            s.contains("__has_value"),
            "mixed contract should hoist __has_value"
        );
        assert!(
            s.contains("NON_PAYABLE_VALUE_RECEIVED"),
            "mixed contract should guard non-payable arms"
        );
    }

    #[test]
    fn struct_without_sol_event_derive_is_ignored() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                #[derive(Debug, Clone)]
                struct Plain {
                    x: u64,
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

        let output = expand_contract(ContractArgs::default(), item)
            .unwrap()
            .to_string();

        assert!(
            !output.contains("Plain :: abi_item") && !output.contains("Plain::abi_item"),
            "Non-event structs should not leak into abi-gen output"
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
    fn slot_fields_generate_construction() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract {
                    #[slot(0)]
                    counter: Lazy<U256>,
                    #[slot(1)]
                    balances: Mapping<Address, U256>,
                }
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

        // Each slot field is constructed with StorageKey::from_slot(N) and host.clone()
        assert!(
            output.contains("from_slot (0u64") && output.contains("from_slot (1u64"),
            "Slot fields should produce from_slot construction.\n\
             Expanded output:\n{output}"
        );
        assert!(
            output.contains("host . clone ()"),
            "Slot fields should receive a host clone.\n\
             Expanded output:\n{output}"
        );

        // #[slot(N)] attributes should not appear in the emitted struct
        assert!(
            !output.contains("# [slot"),
            "Slot attributes should be stripped from the struct output.\n\
             Expanded output:\n{output}"
        );
    }

    #[test]
    fn slot_fields_initialize_in_deploy_and_call() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract {
                    #[slot(0)]
                    counter: Lazy<U256>,
                }
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

        let slot_init_count = output.matches("from_slot (0u64").count();
        assert!(
            slot_init_count >= 2,
            "Slot field should be initialized in both deploy() and call().\n\
             Expanded output:\n{output}"
        );
    }

    #[test]
    fn no_slot_fields_no_storage_construction() {
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
            !output.contains("from_slot"),
            "Unit struct should not produce storage construction"
        );
    }

    #[test]
    fn missing_slot_attr_rejected_for_non_host_fields() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract {
                    counter: Lazy<U256>,
                }
                impl MyContract {
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
            err.contains("must have a `#[slot(N)]` attribute"),
            "Expected missing-slot validation. Got: {err}"
        );
    }

    #[test]
    fn host_field_with_slot_attr_is_rejected() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract {
                    #[slot(0)]
                    host: Lazy<U256>,
                }
                impl MyContract {
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
            err.contains("`host` is a reserved field name"),
            "Expected reserved-host validation. Got: {err}"
        );
    }

    #[test]
    fn duplicate_slot_numbers_rejected() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract {
                    #[slot(0)]
                    a: Lazy<U256>,
                    #[slot(0)]
                    b: Lazy<U256>,
                }
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}
                }
            }
        "#,
        )
        .unwrap();

        let result = expand_contract(ContractArgs::default(), item);
        assert!(result.is_err(), "Should reject duplicate slot numbers");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate slot 0"),
            "Error should mention the duplicate slot. Got: {err}"
        );
    }

    #[test]
    fn cfg_gated_same_name_same_slot_allowed() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract {
                    #[cfg(feature = "v1")]
                    #[slot(0)]
                    data: Lazy<U256>,
                    #[cfg(not(feature = "v1"))]
                    #[slot(0)]
                    data: Mapping<Address, U256>,
                }
                impl MyContract {
                    #[pvm_contract_macros::constructor]
                    pub fn new(&mut self) {}
                }
            }
        "#,
        )
        .unwrap();

        assert!(
            expand_contract(ContractArgs::default(), item).is_ok(),
            "Same name + same slot + both cfg-gated should be allowed"
        );
    }

    #[test]
    fn cfg_gated_different_name_same_slot_rejected() {
        let item: ItemMod = syn::parse_str(
            r#"
            mod my_contract {
                pub struct MyContract {
                    #[cfg(feature = "a")]
                    #[slot(0)]
                    balance_a: Lazy<U256>,
                    #[cfg(feature = "b")]
                    #[slot(0)]
                    balance_b: Lazy<U256>,
                }
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
            "Different names with same slot should be rejected even when cfg-gated"
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

    #[test]
    fn parse_contract_with_receive() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self) {}
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        assert!(parsed.has_receive);
        assert_eq!(parsed.receive_name.unwrap(), "receive");
        assert!(!parsed.receive_returns_result);
    }

    #[test]
    fn parse_contract_with_fallible_receive() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self) -> Result<(), MyError> { Ok(()) }
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        assert!(parsed.has_receive);
        assert!(parsed.receive_returns_result);
    }

    #[test]
    fn parse_contract_rejects_payable_on_receive() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    #[pvm_contract_macros::payable]
                    pub fn receive(&mut self) {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("implicitly payable"), "got: {err}");
    }

    #[test]
    fn parse_contract_rejects_receive_with_ref_self() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&self) {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("&mut self"), "got: {err}");
    }

    #[test]
    fn parse_contract_rejects_receive_with_args() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self, _x: u64) {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("no arguments"), "got: {err}");
    }

    #[test]
    fn parse_contract_rejects_receive_returning_value() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self) -> u64 { 0 }
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("cannot return a value"), "got: {err}");
    }

    #[test]
    fn parse_contract_rejects_duplicate_receive() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn r1(&mut self) {}
                    #[pvm_contract_macros::receive]
                    pub fn r2(&mut self) {}
                }
            }
        };
        let err = match parse_contract(&input, None) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("duplicate"), "got: {err}");
    }

    /// Pretty-print the `pub extern "C" fn call() { ... }` function from an
    /// expanded contract token stream. Used for snapshot-based dispatch tests.
    fn pretty_call_fn(tokens: TokenStream) -> String {
        let file: syn::File = syn::parse2(tokens).expect("expansion parses as syn::File");
        for item in &file.items {
            let syn::Item::Mod(m) = item else { continue };
            let Some((_, items)) = &m.content else {
                continue;
            };
            for inner in items {
                if let syn::Item::Fn(f) = inner
                    && f.sig.ident == "call"
                {
                    let wrapper: syn::File = syn::parse_quote! { #f };
                    return prettyplease::unparse(&wrapper);
                }
            }
        }
        panic!("`fn call` not found in expansion")
    }

    #[test]
    fn receive_emits_empty_calldata_dispatch() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self) {}
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let expected = expect_test::expect![[r##"
            #[cfg(not(feature = "abi-gen"))]
            #[cfg(target_arch = "riscv64")]
            #[polkavm_derive::polkavm_export]
            pub extern "C" fn call() {
                use ::pvm_contract_sdk::pallet_revive_uapi::HostFn as _;
                let host = ::pvm_contract_sdk::Host::new();
                let mut this = C { host };
                let call_data_len = ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_size()
                    as usize;
                let mut call_data = [0u8; 256usize];
                if call_data_len > 256usize {
                    ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                        ::pvm_contract_sdk::ReturnFlags::REVERT,
                        &::pvm_contract_sdk::framework_errors::CALLDATA_TOO_LARGE,
                    );
                }
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::call_data_copy(
                    &mut call_data[..call_data_len],
                    0,
                );
                if call_data_len < 4 {
                    if call_data_len == 0 {
                        this.receive();
                        return;
                    }
                    ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                        ::pvm_contract_sdk::ReturnFlags::REVERT,
                        &::pvm_contract_sdk::framework_errors::NO_SELECTOR,
                    );
                }
                let selector: [u8; 4] = call_data[0..4].try_into().unwrap();
                let input = &call_data[4..call_data_len];
                if route(&mut this, selector, input).is_none() {
                    ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                        ::pvm_contract_sdk::ReturnFlags::REVERT,
                        &::pvm_contract_sdk::framework_errors::UNKNOWN_SELECTOR,
                    );
                }
            }
        "##]];
        expected.assert_eq(&pretty_call_fn(tokens));
    }

    #[test]
    fn contract_without_receive_omits_empty_calldata_check() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::method]
                    pub fn get(&self) -> U256 { U256::ZERO }
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = pretty_call_fn(tokens);
        assert!(
            !s.contains("call_data_len == 0"),
            "contract without #[receive] must not emit empty-calldata branch (size cost); got:\n{s}"
        );
    }

    #[test]
    fn receive_and_fallback_both_emitted_in_dispatch_order() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self) {}

                    #[pvm_contract_macros::fallback]
                    pub fn fallback(&mut self) -> Result<(), MyError> { Ok(()) }
                }
            }
        };
        let parsed = parse_contract(&input, None).unwrap();
        assert!(parsed.has_receive);
        assert!(parsed.has_fallback);

        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = pretty_call_fn(tokens);
        let empty_check_idx = s
            .find("call_data_len == 0")
            .expect("receive arm must check empty calldata");
        let fallback_call_idx = s
            .find("this.fallback")
            .expect("fallback must be invoked too");
        assert!(
            empty_check_idx < fallback_call_idx,
            "receive empty-calldata check must dispatch before fallback path:\n{s}"
        );
    }

    #[test]
    fn receive_with_result_return_emits_revert_path() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self) -> Result<(), MyError> { Ok(()) }
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = pretty_call_fn(tokens);
        assert!(
            s.contains("match this.receive()"),
            "Result-returning receive must use match in dispatch; got:\n{s}"
        );
        assert!(
            s.contains("Ok(())"),
            "Result-returning receive arm must handle Ok branch; got:\n{s}"
        );
    }

    #[test]
    fn receive_dispatch_skips_payable_guard() {
        let input: syn::ItemMod = syn::parse_quote! {
            mod c {
                pub struct C;
                impl C {
                    #[pvm_contract_macros::receive]
                    pub fn receive(&mut self) {}
                }
            }
        };
        let tokens = expand_contract(ContractArgs::default(), input).unwrap();
        let s = pretty_call_fn(tokens);
        assert!(
            !s.contains("__pvm_assert_non_payable"),
            "receive is implicitly payable: call() must not invoke the non-payable guard; got:\n{s}"
        );
    }
}
