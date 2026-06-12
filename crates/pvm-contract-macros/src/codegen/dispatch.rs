use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};

/// Generate boundary-style revert encoding — calls `HostFnImpl::return_value`
/// directly. Used only inside `call()` / `deploy()` boundaries (fallback, and
/// constructor revert paths), which are already riscv64-gated.
pub(super) fn generate_revert_encoding_boundary(use_alloc: bool) -> TokenStream {
    if use_alloc {
        quote! {
            use ::pvm_contract_sdk::SolError;
            let __revert_len = e.encoded_size();
            let mut __revert_buf = alloc::vec![0u8; __revert_len];
            e.encode_to(&mut __revert_buf);
            ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                ::pvm_contract_sdk::ReturnFlags::REVERT, &__revert_buf);
        }
    } else {
        quote! {
            use ::pvm_contract_sdk::SolError;
            let mut __revert_buf = [0u8; 256];
            let __revert_len = e.encode_to(&mut __revert_buf);
            ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                ::pvm_contract_sdk::ReturnFlags::REVERT, &__revert_buf[..__revert_len]);
        }
    }
}

/// Generate dispatch-style revert encoding — calls `host.return_value(REVERT, ...)`
/// on the contract's instance host. On `riscv64` this diverges via the syscall;
/// on host targets it captures into the `MockHost` for the test to inspect.
/// After the call the dispatch arm returns `Some(())` to signal the selector
/// was handled.
fn generate_revert_via_host(use_alloc: bool) -> TokenStream {
    if use_alloc {
        quote! {
            use ::pvm_contract_sdk::SolError;
            let __revert_len = e.encoded_size();
            let mut __revert_buf = alloc::vec![0u8; __revert_len];
            e.encode_to(&mut __revert_buf);
            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                this.host(),
                ::pvm_contract_sdk::ReturnFlags::REVERT,
                &__revert_buf,
            );
            #[allow(unreachable_code)]
            return ::core::option::Option::Some(());
        }
    } else {
        quote! {
            use ::pvm_contract_sdk::SolError;
            let mut __revert_buf = [0u8; 256];
            let __revert_len = e.encode_to(&mut __revert_buf);
            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                this.host(),
                ::pvm_contract_sdk::ReturnFlags::REVERT,
                &__revert_buf[..__revert_len],
            );
            #[allow(unreachable_code)]
            return ::core::option::Option::Some(());
        }
    }
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
    /// When set, the selector is precomputed (e.g. from a `.sol` file).
    pub precomputed_selector: Option<[u8; 4]>,
}

pub(super) struct ParamDecoding {
    /// Expression evaluating to the minimum required input length.
    /// Caller wraps this in the appropriate revert mechanism (boundary
    /// `HostFnImpl::return_value` for constructors, or
    /// `host.return_value(REVERT, ...)` for dispatch arms).
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
    is_constructor: bool,
) -> ParamDecoding {
    let decodes = generate_decode_params(param_types, is_constructor);
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

fn build_selector_const(method: &MethodInfo) -> TokenStream {
    let sel_ident = quote::format_ident!("__SEL_{}", method.fn_name);

    if let Some(selector) = method.precomputed_selector {
        let [s0, s1, s2, s3] = selector;
        quote! {
            const #sel_ident: [u8; 4] = [#s0, #s1, #s2, #s3];
        }
    } else {
        let sig_expr = build_const_signature_expr(method);
        quote! {
            const #sel_ident: [u8; 4] = ::pvm_contract_sdk::const_selector(#sig_expr);
        }
    }
}

fn build_const_signature_expr(method: &MethodInfo) -> TokenStream {
    let fn_name = &method.sol_name;
    let mut parts: Vec<TokenStream> = Vec::new();
    let prefix = format!("{}(", fn_name);
    parts.push(quote! { #prefix });

    for (i, ty) in method.param_types.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        parts.push(quote! { <#ty as ::pvm_contract_sdk::SolEncode>::SOL_NAME });
    }

    parts.push(quote! { ")" });
    quote! { ::pvm_contract_sdk::const_format::concatcp!(#(#parts),*) }
}

/// Size-check wrapped in dispatch-arm style — calls
/// `host.return_value(REVERT, INVALID_CALLDATA)` and returns `Some(())` when
/// the input is too short. On `riscv64` the call diverges; on host targets
/// the test reads the captured return.
fn dispatch_size_check(has_params: bool, min_size_expr: &TokenStream) -> TokenStream {
    if has_params {
        quote! {
            if input.len() < (#min_size_expr) {
                <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                    this.host(),
                    ::pvm_contract_sdk::ReturnFlags::REVERT,
                    &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                );
                #[allow(unreachable_code)]
                return ::core::option::Option::Some(());
            }
        }
    } else {
        quote! {}
    }
}

/// Size-check wrapped in boundary style — calls `HostFnImpl::return_value`
/// directly (riscv64-only). Used by `deploy()` for constructor params.
pub(super) fn boundary_size_check(has_params: bool, min_size_expr: &TokenStream) -> TokenStream {
    if has_params {
        quote! {
            if input.len() < (#min_size_expr) {
                ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                    ::pvm_contract_sdk::ReturnFlags::REVERT,
                    &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA);
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
    let sel_ident = quote::format_ident!("__SEL_{}", method.fn_name);
    let const_def = build_selector_const(method);

    let fn_name = &method.fn_name;
    let decoding = generate_param_decoding(&method.param_names, &method.param_types, false);
    let ParamDecoding {
        min_size_expr,
        decode_statements,
        call_args,
        has_params,
    } = decoding;
    let size_check = dispatch_size_check(has_params, &min_size_expr);
    let has_return = !method.return_types.is_empty();
    let encode_and_return = generate_encode_and_return(&method.return_types, use_alloc);

    let revert_err = generate_revert_via_host(use_alloc);

    let payable_guard = if guard_hoisted || method.mutability == StateMutability::Payable {
        quote! {}
    } else {
        quote! {
            __pvm_assert_value_zero(this.host(), __has_value);
        }
    };

    // Pure methods are associated functions — no `self` receiver — so dispatch
    // them via UFCS (`Self::fn_name`) rather than method-call syntax
    // (`this.fn_name`), which would only work for `&self` / `&mut self`.
    let invoke = if method.mutability == StateMutability::Pure {
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
                    Ok(()) => {
                        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                            this.host(),
                            ::pvm_contract_sdk::ReturnFlags::empty(),
                            &[],
                        );
                        #[allow(unreachable_code)]
                        return ::core::option::Option::Some(());
                    }
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
            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                this.host(),
                ::pvm_contract_sdk::ReturnFlags::empty(),
                &[],
            );
            #[allow(unreachable_code)]
            return ::core::option::Option::Some(());
        }
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
    /// The `route(this, selector, input) -> Option<()>` function.
    pub route_fn: TokenStream,
}

/// `impl Router for mod_name::StructName` block, placed outside the module.
pub struct RouterImpl {
    pub tokens: TokenStream,
}

/// Generate the `route` function and `Router` trait impl for a contract module.
///
/// `route` takes `&mut Contract` and returns `Option<()>`. Each matched
/// dispatch arm calls `this.host().return_value(...)` directly — `-> !` on
/// `riscv64` (terminates execution), `-> ()` on host targets (captures into
/// `MockHost` for tests). The arm then returns `Some(())` (unreachable on
/// `riscv64`, observed by the test harness on host targets). Unmatched
/// selectors return `None`, allowing composition via `Option::or_else` for
/// inheritance / parent-router fallthrough.
///
/// When every method is non-payable the value-transfer guard collapses into a
/// single `__pvm_assert_non_payable()` call before the match. Mixed payability
/// reads `value_transferred` once into `__has_value` and each non-payable arm
/// calls `__pvm_assert_value_zero(host, __has_value)`.
pub fn generate_router(
    methods: &[MethodInfo],
    mod_name: &syn::Ident,
    struct_name: &syn::Ident,
    use_alloc: bool,
) -> (RouteItems, RouterImpl) {
    let all_non_payable = !methods.is_empty()
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

    let prelude = if all_non_payable {
        quote! { __pvm_assert_non_payable(this.host()); }
    } else if any_non_payable {
        quote! {
            let __has_value = ::pvm_contract_sdk::value_transferred_is_nonzero(this.host());
        }
    } else {
        quote! {}
    };

    let route_items = RouteItems {
        route_fn: quote! {
            #[allow(non_upper_case_globals, unreachable_code)]
            pub fn route(
                this: &mut #struct_name,
                selector: [u8; 4],
                input: &[u8],
            ) -> ::core::option::Option<()> {
                use ::pvm_contract_sdk::pallet_revive_uapi::HostFn as _;
                #(#selector_consts)*

                #prelude

                match selector {
                    #(#dispatch_arms)*
                    _ => ::core::option::Option::None,
                }
            }
        },
    };

    let router_impl = RouterImpl {
        tokens: quote! {
            impl ::pvm_contract_sdk::Router for #mod_name::#struct_name {
                fn route(
                    &mut self,
                    selector: [u8; 4],
                    input: &[u8],
                ) -> ::core::option::Option<()> {
                    #mod_name::route(self, selector, input)
                }
            }
        },
    };

    (route_items, router_impl)
}

fn generate_encode_and_return(outputs: &[syn::Type], use_alloc: bool) -> TokenStream {
    if outputs.is_empty() {
        return quote! {
            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                this.host(),
                ::pvm_contract_sdk::ReturnFlags::empty(),
                &[],
            );
            #[allow(unreachable_code)]
            return ::core::option::Option::Some(());
        };
    }

    if use_alloc {
        generate_alloc_encode_and_return(outputs)
    } else {
        generate_static_encode_and_return(outputs)
    }
}

fn generate_static_encode_and_return(outputs: &[syn::Type]) -> TokenStream {
    if outputs.len() == 1 {
        let ty = &outputs[0];
        return quote! {{
            const { assert!(
                !<#ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC,
                "dynamic types (String, Vec, Bytes) require allocator = \"pico\" or \"bump\""
            ) };
            const __LEN: usize = <#ty as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE;
            let mut __buf = [0u8; __LEN];
            <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                this.host(),
                ::pvm_contract_sdk::ReturnFlags::empty(),
                &__buf,
            );
            #[allow(unreachable_code)]
            return ::core::option::Option::Some(());
        }};
    }

    let tuple_ty = quote! { (#(#outputs,)*) };
    quote! {{
        const { assert!(
            !<#tuple_ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC,
            "dynamic return types require allocator = \"pico\" or \"bump\""
        ) };
        const __LEN: usize = <#tuple_ty as ::pvm_contract_sdk::StaticEncodedLen>::ENCODED_SIZE;
        let mut __buf = [0u8; __LEN];
        <#tuple_ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __buf);
        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
            this.host(),
            ::pvm_contract_sdk::ReturnFlags::empty(),
            &__buf,
        );
        #[allow(unreachable_code)]
        return ::core::option::Option::Some(());
    }}
}

fn generate_alloc_encode_and_return(outputs: &[syn::Type]) -> TokenStream {
    if outputs.len() == 1 {
        let ty = &outputs[0];
        return quote! {{
            let __len = <#ty as ::pvm_contract_sdk::SolEncode>::encode_len(&result);
            let mut __dyn_buf: alloc::vec::Vec<u8>;
            let mut __static_buf: [u8; <#ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE];
            let __data: &[u8] = if <#ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
                __dyn_buf = alloc::vec![0u8; __len];
                <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __dyn_buf);
                &__dyn_buf
            } else {
                __static_buf = [0u8; <#ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE];
                <#ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __static_buf[..__len]);
                &__static_buf[..__len]
            };
            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                this.host(),
                ::pvm_contract_sdk::ReturnFlags::empty(),
                __data,
            );
            #[allow(unreachable_code)]
            return ::core::option::Option::Some(());
        }};
    }

    let tuple_ty = quote! { (#(#outputs,)*) };
    quote! {{
        let __len = <#tuple_ty as ::pvm_contract_sdk::SolEncode>::encode_len(&result);
        let mut __dyn_buf: alloc::vec::Vec<u8>;
        let mut __static_buf: [u8; <#tuple_ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE];
        let __data: &[u8] = if <#tuple_ty as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
            __dyn_buf = alloc::vec![0u8; __len];
            <#tuple_ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __dyn_buf);
            &__dyn_buf
        } else {
            __static_buf = [0u8; <#tuple_ty as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE];
            <#tuple_ty as ::pvm_contract_sdk::SolEncode>::encode_to(&result, &mut __static_buf[..__len]);
            &__static_buf[..__len]
        };
        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
            this.host(),
            ::pvm_contract_sdk::ReturnFlags::empty(),
            __data,
        );
        #[allow(unreachable_code)]
        return ::core::option::Option::Some(());
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
            precomputed_selector: Some([0xde, 0xad, 0xbe, 0xef]),
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
                            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                                this.host(),
                                ::pvm_contract_sdk::ReturnFlags::REVERT,
                                &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                            );
                            #[allow(unreachable_code)] return ::core::option::Option::Some(());
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
                        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                            this.host(),
                            ::pvm_contract_sdk::ReturnFlags::empty(),
                            &[],
                        );
                        #[allow(unreachable_code)] return ::core::option::Option::Some(());
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
                            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                                this.host(),
                                ::pvm_contract_sdk::ReturnFlags::REVERT,
                                &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                            );
                            #[allow(unreachable_code)] return ::core::option::Option::Some(());
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
                        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                            this.host(),
                            ::pvm_contract_sdk::ReturnFlags::empty(),
                            &[],
                        );
                        #[allow(unreachable_code)] return ::core::option::Option::Some(());
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
                            <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                                this.host(),
                                ::pvm_contract_sdk::ReturnFlags::REVERT,
                                &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                            );
                            #[allow(unreachable_code)] return ::core::option::Option::Some(());
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
                        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                            this.host(),
                            ::pvm_contract_sdk::ReturnFlags::empty(),
                            &[],
                        );
                        #[allow(unreachable_code)] return ::core::option::Option::Some(());
                    }
                    _ => {}
                }
            }
        "#]];
        expected.assert_eq(&pretty(&arm));
    }
}
