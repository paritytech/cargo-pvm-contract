use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};
use crate::utils::build_method_signature_expr;

/// Generate a **diverging** revert — encode the error `e`, then
/// `host.revert(..)` (`-> !`: the syscall on `riscv64`; a recorded unwind on
/// host so `expect_revert` catches it). Because it diverges, no trailing
/// `return` is needed. The revert length is `encode_to`'s returned byte count,
/// not `encoded_size()`, so an impl that writes fewer bytes can't forward
/// trailing garbage.
///
/// The only thing that varies between call sites is **where the payload is
/// encoded**, selected by `dst`:
/// - [`RevertBuf::Out`] — reuse the dispatch arm's caller-owned `out`
///   ([`OutSink`]). Used by `#[method]` `Err(e)` arms inside `route()`.
/// - [`RevertBuf::Local`] — a fresh local buffer (`Vec` under alloc, a `[u8;
///   256]` otherwise). Used at the `deploy()` / `#[receive]` / `#[fallback]`
///   boundaries, which have no `out` in scope.
///
/// Those boundaries are **deliberately not** on the `Outcome`-return path: they
/// live in `#[cfg(target_arch = "riscv64")]` `deploy()`/`call()` (never
/// host-reachable, so `Outcome` would add no testability), and receive/fallback
/// *success* is a bare `return;` whose semantics we don't want to change.
///
/// # Assumes in scope
/// `e` (the error) and `this` (the contract) always; plus `out` (an [`OutSink`])
/// when `dst == RevertBuf::Out`. These are established by the enclosing generated
/// `route()` / `deploy()` / `call()` body.
#[derive(Clone, Copy)]
pub(super) enum RevertBuf {
    /// Encode into the arm's caller-owned `out` buffer.
    Out,
    /// Encode into a fresh local buffer (no `out` at this site).
    Local,
}

pub(super) fn generate_revert(dst: RevertBuf, use_alloc: bool) -> TokenStream {
    // Capacity to reserve: the exact encoded size under alloc, or the fixed
    // 256-byte cap in no-alloc mode.
    let cap = if use_alloc {
        quote! { e.encoded_size() }
    } else {
        quote! { 256 }
    };
    let (encode, data) = match dst {
        RevertBuf::Out => (
            quote! {
                let __revert_len = {
                    let __revert_buf = out.reserve(#cap);
                    e.encode_to(__revert_buf)
                };
            },
            quote! { out.view(__revert_len) },
        ),
        RevertBuf::Local if use_alloc => (
            quote! {
                let mut __revert_buf = alloc::vec![0u8; #cap];
                let __revert_len = e.encode_to(&mut __revert_buf);
            },
            quote! { &__revert_buf[..__revert_len] },
        ),
        RevertBuf::Local => (
            quote! {
                let mut __revert_buf = [0u8; #cap];
                let __revert_len = e.encode_to(&mut __revert_buf);
            },
            quote! { &__revert_buf[..__revert_len] },
        ),
    };
    // Under alloc the buffer is sized to exactly `encoded_size()`, so a correct
    // `SolError::encode_to` must write that many bytes; assert it in debug builds
    // to catch an impl that under-writes (which would forward stale/zero bytes).
    // No-alloc caps at 256 where a long `RevertString` legitimately truncates, so
    // the equality does not hold there and the assert is omitted.
    let assert_len = if use_alloc {
        quote! {
            debug_assert!(
                __revert_len == e.encoded_size(),
                "SolError::encode_to wrote a different length than encoded_size() reported",
            );
        }
    } else {
        quote! {}
    };
    quote! {{
        use ::pvm_contract_sdk::SolError;
        #encode
        #assert_len
        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
            this.host(),
            #data,
        )
    }}
}

/// Solidity's state mutability classifications. Mutually exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateMutability {
    Pure,
    View,
    NonPayable,
    Payable,
}

impl StateMutability {
    pub fn as_abi_str(self) -> &'static str {
        match self {
            StateMutability::Pure => "pure",
            StateMutability::View => "view",
            StateMutability::NonPayable => "nonpayable",
            StateMutability::Payable => "payable",
        }
    }
}

pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub sol_name: String,
    pub param_names: Vec<syn::Ident>,
    pub param_types: Vec<syn::Type>,
    pub return_types: Vec<syn::Type>,
    pub returns_result: bool,
    pub mutability: StateMutability,
    /// `#[non_reentrant]`: emit a reentrancy guard.
    pub is_non_reentrant: bool,
    /// `None` for an inherent `#[method]`; `Some(path)` for a method folded from
    /// an `impl Path for Contract` block via `implements(...)`. When set,
    /// the dispatch arm invokes the method through a fully-qualified trait call
    /// `<Struct as Path>::method(this, ...)` so it runs the contract's own
    /// trait-impl body (overrides) and can't be shadowed by an inherent method.
    pub trait_path: Option<syn::Path>,
}

pub(super) struct ParamDecoding {
    /// Expression evaluating to the minimum required input length.
    /// Caller wraps this in a size check that reverts via `host.revert(...)`
    /// on underflow (see [`size_check`]) — same mechanism for both dispatch
    /// arms and the `deploy()` constructor boundary.
    pub min_size_expr: TokenStream,
    pub decode_statements: Vec<TokenStream>,
    pub call_args: Vec<TokenStream>,
    /// True when decoding is non-empty (i.e. there are params to check/decode).
    pub has_params: bool,
}

/// Generate parameter decoding for a method: input size check, decode
/// statements that bind each ABI param to a local, and the `call_args` list
/// used when invoking the user function.
pub(super) fn generate_param_decoding(
    param_names: &[syn::Ident],
    param_types: &[syn::Type],
) -> ParamDecoding {
    let decodes = generate_decode_params(param_types);
    let min_size_expr = calculate_min_input_size(param_types);
    let has_params = !param_types.is_empty();

    let offset_init = if has_params {
        quote! { let mut __decode_offset: usize = 0; }
    } else {
        quote! {}
    };

    let decode_statements = std::iter::once(offset_init)
        .chain(
            param_names
                .iter()
                .zip(decodes.iter())
                .map(|(name, decode)| {
                    quote! { let #name = #decode; }
                }),
        )
        .collect();

    let call_args = param_names
        .iter()
        .map(|name| quote!(::core::convert::Into::into(#name)))
        .collect();

    ParamDecoding {
        min_size_expr,
        decode_statements,
        call_args,
        has_params,
    }
}

/// The selector-const identifier for a method. Folded methods are namespaced by
/// their trait's last segment (`__SEL_<Trait>_<fn>`) so two inherited traits with
/// a same-named method don't collide at the const-ident level.
fn selector_const_ident(method: &MethodInfo) -> syn::Ident {
    match &method.trait_path {
        Some(path) => {
            let trait_last = path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            quote::format_ident!("__SEL_{}_{}", trait_last, method.fn_name)
        }
        None => quote::format_ident!("__SEL_{}", method.fn_name),
    }
}

fn build_selector_const(method: &MethodInfo) -> TokenStream {
    let sel_ident = selector_const_ident(method);

    let sig_expr = build_method_signature_expr(&method.sol_name, &method.param_types);
    quote! {
        const #sel_ident: [u8; 4] = ::pvm_contract_sdk::const_selector(#sig_expr);
    }
}

/// Input size-check — calls `host.revert(INVALID_CALLDATA)` when the input is
/// too short. Used by both dispatch arms (`route()`) and the `deploy()`
/// constructor boundary. `revert` is `-> !`: on `riscv64` it diverges via the
/// syscall; on host targets it unwinds so `expect_revert` catches it. No
/// trailing `return` is needed.
pub(super) fn size_check(has_params: bool, min_size_expr: &TokenStream) -> TokenStream {
    if has_params {
        quote! {
            if input.len() < (#min_size_expr) {
                <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
                    this.host(),
                    &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                );
            }
        }
    } else {
        quote! {}
    }
}

pub fn generate_dispatch_arm(
    method: &MethodInfo,
    struct_name: &syn::Ident,
    use_alloc: bool,
    guard_hoisted: bool,
) -> (TokenStream, TokenStream) {
    let sel_ident = selector_const_ident(method);
    let const_def = build_selector_const(method);

    let fn_name = &method.fn_name;
    let decoding = generate_param_decoding(&method.param_names, &method.param_types);
    let ParamDecoding {
        min_size_expr,
        decode_statements,
        mut call_args,
        has_params,
    } = decoding;
    // A folded trait method is invoked through a fully-qualified trait call
    // `<Struct as Trait>::method`, where the receiver is passed as the first
    // positional argument, so prepend `this` to the decoded args. (`&mut this`
    // coerces to `&self` for view methods.) Inherent methods keep method-call
    // syntax and need no self arg.
    if method.trait_path.is_some() {
        call_args.insert(0, quote! { this });
    }
    let size_check = size_check(has_params, &min_size_expr);
    let has_return = !method.return_types.is_empty();
    let encode_and_return = generate_encode_and_return(&method.return_types, use_alloc);

    let revert_err = generate_revert(RevertBuf::Out, use_alloc);

    let payable_guard = if guard_hoisted || method.mutability == StateMutability::Payable {
        quote! {}
    } else {
        quote! {
            __pvm_assert_value_zero(this.host(), __has_value);
        }
    };

    // Folded trait methods dispatch through a fully-qualified trait call
    // `<Struct as Trait>::fn_name(this, ...)` — runs the contract's own trait-impl
    // body (so overrides work) and can't be shadowed by an inherent method.
    // Pure methods are associated functions — no `self` receiver — so dispatch
    // them via a fully-qualified call (`Self::fn_name`) rather than method-call
    // syntax (`this.fn_name`), which would only work for `&self` / `&mut self`.
    let invoke = if let Some(trait_path) = &method.trait_path {
        quote! { <#struct_name as #trait_path>::#fn_name }
    } else if method.mutability == StateMutability::Pure {
        quote! { #struct_name::#fn_name }
    } else {
        quote! { this.#fn_name }
    };

    let body = if method.returns_result {
        if has_return {
            quote! {
                match #invoke(#(#call_args),*) {
                    Ok(result) => { #encode_and_return }
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        } else {
            quote! {
                match #invoke(#(#call_args),*) {
                    Ok(()) => ::pvm_contract_sdk::Outcome::Return(0),
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        }
    } else if has_return {
        quote! {
            let result = #invoke(#(#call_args),*);
            #encode_and_return
        }
    } else {
        quote! {
            #invoke(#(#call_args),*);
            ::pvm_contract_sdk::Outcome::Return(0)
        }
    };

    // `#[non_reentrant]`: wrap the body with the guard, with the mode inferred
    // from the receiver below. This emits an explicit unlock after the body for
    // the normal-return path; a body that diverges via a raw `return_value`
    // skips it and is instead released inside `return_value` itself (see
    // `pvm-contract-types::reentrancy`). `Drop` can't cover either path:
    // `return_value` diverges without unwinding, so no destructor runs.
    //
    // On re-entry (lock already held) revert with the OZ-compatible error. Like
    // every revert in dispatch it diverges via `host.revert(...)` (`-> !`), so no
    // trailing `return` is needed, and it never clears the outer frame's lock
    // (this frame's `REENTRANCY_LOCK_HELD` flag is `false` — it never locked).
    let revert_if_locked = quote! {
        if ::pvm_contract_sdk::__reentrancy_is_locked(this.host()) {
            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
                this.host(),
                &<::pvm_contract_sdk::ReentrancyGuardReentrantCall as ::pvm_contract_sdk::SolError>::SELECTOR,
            );
        }
    };

    let body = if method.is_non_reentrant {
        match method.mutability {
            // `&self` read-only check (`nonReentrantView`): revert if a guarded
            // section is in progress; no lock/unlock, body unchanged.
            StateMutability::View => quote! {
                #revert_if_locked
                #body
            },
            // `&mut self` full guard: check, lock, run body, unlock before returning.
            StateMutability::NonPayable | StateMutability::Payable => {
                if method.returns_result {
                    if has_return {
                        quote! {
                            #revert_if_locked
                            ::pvm_contract_sdk::__reentrancy_lock(this.host());
                            let __r = #invoke(#(#call_args),*);
                            ::pvm_contract_sdk::__reentrancy_unlock(this.host());
                            match __r {
                                Ok(result) => { #encode_and_return }
                                Err(e) => { #revert_err }
                            }
                        }
                    } else {
                        quote! {
                            #revert_if_locked
                            ::pvm_contract_sdk::__reentrancy_lock(this.host());
                            let __r = #invoke(#(#call_args),*);
                            ::pvm_contract_sdk::__reentrancy_unlock(this.host());
                            match __r {
                                Ok(()) => ::pvm_contract_sdk::Outcome::Return(0),
                                Err(e) => { #revert_err }
                            }
                        }
                    }
                } else if has_return {
                    quote! {
                        #revert_if_locked
                        ::pvm_contract_sdk::__reentrancy_lock(this.host());
                        let result = #invoke(#(#call_args),*);
                        ::pvm_contract_sdk::__reentrancy_unlock(this.host());
                        #encode_and_return
                    }
                } else {
                    quote! {
                        #revert_if_locked
                        ::pvm_contract_sdk::__reentrancy_lock(this.host());
                        #invoke(#(#call_args),*);
                        ::pvm_contract_sdk::__reentrancy_unlock(this.host());
                        ::pvm_contract_sdk::Outcome::Return(0)
                    }
                }
            }
            // Pure has no receiver/host and is rejected at parse time; passthrough.
            StateMutability::Pure => body,
        }
    } else {
        body
    };

    let match_arm = quote! {
        #sel_ident => {
            #payable_guard
            #size_check
            #(#decode_statements)*
            #body
        }
    };

    (const_def, match_arm)
}

/// Items generated inside the contract module for routing.
pub struct RouteItems {
    /// `pub const MAX_RETURN_LEN` — the caller-owned output buffer size. Emitted
    /// separately so the caller (`contract.rs`) can `#[cfg]`-gate it alongside
    /// `route_fn` (a single `#[cfg]` attribute only covers one item).
    pub max_return_const: TokenStream,
    /// The `route(this, selector, input, out) -> Outcome` function.
    pub route_fn: TokenStream,
}

/// `impl Router for mod_name::StructName` block, placed outside the module.
pub struct RouterImpl {
    pub tokens: TokenStream,
}

/// Generate the `route` function, its `MAX_RETURN_LEN` const, and the `Router`
/// trait impl for a contract module.
///
/// `route(this, selector, input, out) -> Outcome`. A matched arm that succeeds
/// encodes its result into the caller-owned `out` buffer and evaluates to
/// `Outcome::Return(len)`; an unmatched selector yields `Outcome::Unhandled`.
/// The single `finalize_outcome` exit (in `call()`) lowers a `Return` to the
/// `return_value` success door. Reverts never become an `Outcome`: a method's
/// own `Err(e)` (see [`generate_revert`]) and every framework abort — the
/// size check, the malformed-calldata decode `let-else`, the payable guard —
/// diverge through `host.revert(...)` (`-> !`) at the point they occur.
///
/// For no-alloc contracts a `pub const MAX_RETURN_LEN` is emitted so `call()`
/// (and any hand-written composed router) can size the fixed output buffer to
/// `max(256, max method-return ENCODED_SIZE)`. Alloc contracts grow a `Vec`
/// instead and need no const.
///
/// When every method is non-payable the value-transfer guard collapses into a
/// single `__pvm_assert_non_payable()` call before the match. Mixed payability
/// reads `value_transferred` once into `__has_value` and each non-payable arm
/// calls `__pvm_assert_value_zero(host, __has_value)`.
///
/// A **payable `#[fallback]`** forces the per-arm shape even when every named
/// method is non-payable: the hoisted assert runs before the `match`, so an
/// unmatched value-bearing call would otherwise revert here (in `route()`'s
/// prelude) before reaching the fallback — which is supposed to accept the
/// value. Per-arm guards fire only on matched non-payable methods, leaving the
/// `Outcome::Unhandled` → payable-fallback path free of the pre-assert.
pub fn generate_router(
    methods: &[MethodInfo],
    mod_name: &syn::Ident,
    struct_name: &syn::Ident,
    use_alloc: bool,
    fallback_is_payable: bool,
) -> (RouteItems, RouterImpl) {
    let all_non_payable = !fallback_is_payable
        && !methods.is_empty()
        && methods
            .iter()
            .all(|m| m.mutability != StateMutability::Payable);
    let any_non_payable = methods
        .iter()
        .any(|m| m.mutability != StateMutability::Payable);

    let (selector_consts, dispatch_arms): (Vec<_>, Vec<_>) = methods
        .iter()
        .map(|m| generate_dispatch_arm(m, struct_name, use_alloc, all_non_payable))
        .unzip();

    // Selector-collision guard. Two dispatched methods sharing a 4-byte
    // selector — an inherent `#[method]` vs a folded interface method, two folded
    // methods, or a genuine keccak clash — would leave the second `match` arm
    // dead (`unreachable_patterns`, only a warning). Turn it into a hard compile
    // error. Done at const-eval, not macro-time, so custom parameter types (whose
    // `SOL_NAME` is unknown until const-eval) are covered. The `__SEL_*` consts
    // are emitted below; compare them pairwise as big-endian u32s (both
    // `from_be_bytes` and `!=` are const).
    let collision_guard = {
        let sel_idents: Vec<_> = methods.iter().map(selector_const_ident).collect();
        let mut pairs = Vec::new();
        for i in 0..sel_idents.len() {
            for j in (i + 1)..sel_idents.len() {
                let a = &sel_idents[i];
                let b = &sel_idents[j];
                pairs.push(quote! {
                    ::core::primitive::u32::from_be_bytes(#a)
                        != ::core::primitive::u32::from_be_bytes(#b)
                });
            }
        }
        if pairs.is_empty() {
            quote! {}
        } else {
            quote! {
                const _: () = ::core::assert!(
                    #(#pairs)&&*,
                    "selector collision: two dispatched methods share the same 4-byte selector \
                     (an inherent method and a folded interface method, or two folded methods). \
                     Rename one with #[selector(name = \"...\")]"
                );
            }
        }
    };

    let prelude = if all_non_payable {
        quote! { __pvm_assert_non_payable(this.host()); }
    } else if any_non_payable {
        quote! {
            let __has_value = ::pvm_contract_sdk::value_transferred_is_nonzero(this.host());
        }
    } else {
        quote! {}
    };

    // Compile-time size of the caller-owned inline output buffer, one per module
    // (also usable for composition). No-alloc: the max static return
    // `ENCODED_SIZE`, floored at 256 so the fixed error path (`reserve(256)`)
    // always fits. Alloc: the max return `HEAD_SIZE` (valid for dynamic types
    // too — 32 for the offset word); a return whose runtime length exceeds this
    // spills to the heap, so no 256 floor is needed. This sizes the inline
    // buffer to exactly what static returns need, matching the pre-unification
    // per-arm stack buffer instead of a blanket 256.
    let max_return_const = {
        let size_exprs: Vec<TokenStream> = methods
            .iter()
            .filter(|m| !m.return_types.is_empty())
            .map(|m| {
                let tys = &m.return_types;
                let ty = if tys.len() == 1 {
                    let t = &tys[0];
                    quote! { #t }
                } else {
                    quote! { (#(#tys,)*) }
                };
                if use_alloc {
                    quote! { <#ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE }
                } else {
                    quote! { <#ty as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE }
                }
            })
            .collect();
        let floor: usize = if use_alloc { 0 } else { 256 };
        quote! {
            #[allow(clippy::identity_op)]
            pub const MAX_RETURN_LEN: usize = {
                let mut __m: usize = #floor;
                #(
                    {
                        let __s: usize = #size_exprs;
                        if __s > __m { __m = __s; }
                    }
                )*
                __m
            };
        }
    };

    let route_items = RouteItems {
        max_return_const,
        route_fn: quote! {
            #[allow(non_upper_case_globals)]
            pub fn route<__B: ::pvm_contract_sdk::OutSink>(
                this: &mut #struct_name,
                selector: [u8; 4],
                input: &[u8],
                out: &mut __B,
            ) -> ::pvm_contract_sdk::Outcome {
                use ::pvm_contract_sdk::pallet_revive_uapi::HostFn as _;
                #(#selector_consts)*
                #collision_guard

                #prelude

                match selector {
                    #(#dispatch_arms)*
                    _ => ::pvm_contract_sdk::Outcome::Unhandled,
                }
            }
        },
    };

    let router_impl = RouterImpl {
        tokens: quote! {
            impl ::pvm_contract_sdk::Router for #mod_name::#struct_name {
                fn route<__B: ::pvm_contract_sdk::OutSink>(
                    &mut self,
                    selector: [u8; 4],
                    input: &[u8],
                    out: &mut __B,
                ) -> ::pvm_contract_sdk::Outcome {
                    #mod_name::route(self, selector, input, out)
                }
            }
        },
    };

    (route_items, router_impl)
}

/// Encode a method's return value into the caller-owned `out` buffer and
/// evaluate to `Outcome::Return(len)`. Empty returns write nothing and yield
/// `Outcome::Return(0)`.
fn generate_encode_and_return(outputs: &[syn::Type], use_alloc: bool) -> TokenStream {
    if outputs.is_empty() {
        return quote! { ::pvm_contract_sdk::Outcome::Return(0) };
    }

    if use_alloc {
        generate_alloc_encode_and_return(outputs)
    } else {
        generate_static_encode_and_return(outputs)
    }
}

fn generate_static_encode_and_return(outputs: &[syn::Type]) -> TokenStream {
    let (ty, dyn_msg): (TokenStream, &str) = if outputs.len() == 1 {
        let ty = &outputs[0];
        (
            quote! { #ty },
            "dynamic types (String, Vec, Bytes) require allocator = \"pico\" or \"bump\"",
        )
    } else {
        (
            quote! { (#(#outputs,)*) },
            "dynamic return types require allocator = \"pico\" or \"bump\"",
        )
    };
    quote! {{
        const { assert!(!<#ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC, #dyn_msg) };
        const __LEN: usize = <#ty as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE;
        let __buf = out.reserve(__LEN);
        <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, __buf);
        ::pvm_contract_sdk::Outcome::Return(__LEN)
    }}
}

fn generate_alloc_encode_and_return(outputs: &[syn::Type]) -> TokenStream {
    let ty: TokenStream = if outputs.len() == 1 {
        let ty = &outputs[0];
        quote! { #ty }
    } else {
        quote! { (#(#outputs,)*) }
    };
    quote! {{
        let __len = <#ty as ::pvm_contract_sdk::SolEncode>::encode_len(&result);
        let __buf = out.reserve(__len);
        <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, __buf);
        ::pvm_contract_sdk::Outcome::Return(__len)
    }}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pretty(ts: &TokenStream) -> String {
        let file: syn::File = syn::parse2(quote! {
            fn __w(selector: [u8; 4], input: &[u8], this: &mut Contract) {
                match selector {
                    #ts
                    _ => {}
                }
            }
        })
        .expect("dispatch arm parses inside a match expression");
        prettyplease::unparse(&file)
    }

    fn sample_method(name: &str, mutability: StateMutability) -> MethodInfo {
        MethodInfo {
            fn_name: quote::format_ident!("{name}"),
            sol_name: name.to_string(),
            param_names: vec![quote::format_ident!("to")],
            param_types: vec![syn::parse_quote!(Address)],
            return_types: vec![],
            returns_result: false,
            mutability,
            is_non_reentrant: false,
            trait_path: None,
        }
    }

    #[test]
    fn non_payable_arm_emits_value_zero_assert() {
        let m = sample_method("transfer", StateMutability::NonPayable);
        let struct_name: syn::Ident = syn::parse_quote!(Contract);
        let (_, arm) = generate_dispatch_arm(&m, &struct_name, false, false);
        let expected = expect_test::expect![[r#"
            fn __w(selector: [u8; 4], input: &[u8], this: &mut Contract) {
                match selector {
                    __SEL_transfer => {
                        __pvm_assert_value_zero(this.host(), __has_value);
                        if input.len() < (0 + <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE)
                        {
                            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
                                this.host(),
                                &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                            );
                        }
                        let mut __decode_offset: usize = 0;
                        let to = {
                            let __value = unsafe {
                                <Address as ::pvm_contract_sdk::StaticDecode>::decode_unchecked(
                                    &input,
                                    __decode_offset,
                                )
                            };
                            __decode_offset += <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
                            __value
                        };
                        this.transfer(::core::convert::Into::into(to));
                        ::pvm_contract_sdk::Outcome::Return(0)
                    }
                    _ => {}
                }
            }
        "#]];
        expected.assert_eq(&pretty(&arm));
    }

    #[test]
    fn payable_arm_omits_value_zero_assert() {
        let m = sample_method("deposit", StateMutability::Payable);
        let struct_name: syn::Ident = syn::parse_quote!(Contract);
        let (_, arm) = generate_dispatch_arm(&m, &struct_name, false, false);
        let expected = expect_test::expect![[r#"
            fn __w(selector: [u8; 4], input: &[u8], this: &mut Contract) {
                match selector {
                    __SEL_deposit => {
                        if input.len() < (0 + <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE)
                        {
                            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
                                this.host(),
                                &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                            );
                        }
                        let mut __decode_offset: usize = 0;
                        let to = {
                            let __value = unsafe {
                                <Address as ::pvm_contract_sdk::StaticDecode>::decode_unchecked(
                                    &input,
                                    __decode_offset,
                                )
                            };
                            __decode_offset += <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
                            __value
                        };
                        this.deposit(::core::convert::Into::into(to));
                        ::pvm_contract_sdk::Outcome::Return(0)
                    }
                    _ => {}
                }
            }
        "#]];
        expected.assert_eq(&pretty(&arm));
    }

    #[test]
    fn hoisted_non_payable_arm_omits_value_zero_assert() {
        let m = sample_method("transfer", StateMutability::NonPayable);
        let struct_name: syn::Ident = syn::parse_quote!(Contract);
        let (_, arm) = generate_dispatch_arm(&m, &struct_name, false, true);
        let expected = expect_test::expect![[r#"
            fn __w(selector: [u8; 4], input: &[u8], this: &mut Contract) {
                match selector {
                    __SEL_transfer => {
                        if input.len() < (0 + <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE)
                        {
                            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::revert(
                                this.host(),
                                &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                            );
                        }
                        let mut __decode_offset: usize = 0;
                        let to = {
                            let __value = unsafe {
                                <Address as ::pvm_contract_sdk::StaticDecode>::decode_unchecked(
                                    &input,
                                    __decode_offset,
                                )
                            };
                            __decode_offset += <Address as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
                            __value
                        };
                        this.transfer(::core::convert::Into::into(to));
                        ::pvm_contract_sdk::Outcome::Return(0)
                    }
                    _ => {}
                }
            }
        "#]];
        expected.assert_eq(&pretty(&arm));
    }
}
