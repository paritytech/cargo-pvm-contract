use proc_macro2::TokenStream;
use quote::quote;

use super::decode::{calculate_min_input_size, generate_decode_params};

/// Generate boundary-style revert encoding — calls `HostFnImpl::return_value`
/// directly. Used only inside `call()` / `deploy()` boundaries (fallback, and
/// constructor revert paths), which are already riscv64-gated.
pub(super) fn generate_revert_encoding_boundary(use_alloc: bool) -> TokenStream {
    if use_alloc {
        quote! {
            let __revert_len = ::pvm_contract_sdk::SolRevert::revert_data_len(&e);
            let mut __revert_buf = alloc::vec![0u8; __revert_len];
            ::pvm_contract_sdk::SolRevert::revert_data(&e, &mut __revert_buf);
            ::pvm_contract_sdk::pallet_revive_uapi::HostFnImpl::return_value(
                ::pvm_contract_sdk::ReturnFlags::REVERT, &__revert_buf);
        }
    } else {
        quote! {
            let mut __revert_buf = [0u8; 256];
            let __revert_len = ::pvm_contract_sdk::SolRevert::revert_data(&e, &mut __revert_buf);
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
            let __revert_len = ::pvm_contract_sdk::SolRevert::revert_data_len(&e);
            let mut __revert_buf = alloc::vec![0u8; __revert_len];
            ::pvm_contract_sdk::SolRevert::revert_data(&e, &mut __revert_buf);
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
            let mut __revert_buf = [0u8; 256];
            let __revert_len = ::pvm_contract_sdk::SolRevert::revert_data(&e, &mut __revert_buf);
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

pub struct MethodInfo {
    pub fn_name: syn::Ident,
    pub sol_name: String,
    pub param_names: Vec<syn::Ident>,
    pub param_types: Vec<syn::Type>,
    pub return_types: Vec<syn::Type>,
    pub returns_result: bool,
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

pub fn generate_dispatch_arm(method: &MethodInfo, use_alloc: bool) -> (TokenStream, TokenStream) {
    let sel_ident = quote::format_ident!("__SEL_{}", method.fn_name);
    let const_def = build_selector_const(method);

    let fn_name = &method.fn_name;
    let decoding = generate_param_decoding(&method.param_names, &method.param_types);
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

    let body = if method.returns_result {
        if has_return {
            quote! {
                match this.#fn_name(#(#call_args),*) {
                    Ok(result) => { #encode_and_return }
                    Err(e) => {
                        #revert_err
                    }
                }
            }
        } else {
            quote! {
                match this.#fn_name(#(#call_args),*) {
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
            let result = this.#fn_name(#(#call_args),*);
            #encode_and_return
        }
    } else {
        quote! {
            this.#fn_name(#(#call_args),*);
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

/// `impl Router<Host> for mod_name::StructName` block, placed outside the module.
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
pub fn generate_router(
    methods: &[MethodInfo],
    mod_name: &syn::Ident,
    struct_name: &syn::Ident,
    use_alloc: bool,
) -> (RouteItems, RouterImpl) {
    let (selector_consts, dispatch_arms): (Vec<_>, Vec<_>) = methods
        .iter()
        .map(|m| generate_dispatch_arm(m, use_alloc))
        .unzip();

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

                match selector {
                    #(#dispatch_arms)*
                    _ => ::core::option::Option::None,
                }
            }
        },
    };

    let router_impl = RouterImpl {
        tokens: quote! {
            impl ::pvm_contract_sdk::Router<::pvm_contract_sdk::Host>
                for #mod_name::#struct_name
            {
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
    // Single `host.return_value(...)` call site shared between static and
    // dynamic returns. Each branch fills a different buffer (heap Vec for
    // dynamic, stack array for static) and exposes the encoded bytes via a
    // shared `&[u8]`. LLVM DCEs the dead branch after monomorphization on
    // `IS_DYNAMIC` (a const bool), and consolidating the syscall site cuts
    // the per-arm prologue/epilogue cost vs. emitting two separate calls.
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
