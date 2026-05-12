use proc_macro2::TokenStream;
use quote::quote;

use crate::signature::SolType;

/// Generate decode expressions for each parameter type.
///
/// Each expression reads from `input` at `__decode_offset` and advances
/// `__decode_offset` by the type's `HEAD_SIZE`. The caller must emit
/// `let mut __decode_offset: usize = 0;` before using these.
pub fn generate_decode_params(types: &[syn::Type], is_constructor: bool) -> Vec<TokenStream> {
    let ret = if is_constructor {
        quote! {
            #[allow(unreachable_code)]
            return ();
        }
    } else {
        quote! {
            #[allow(unreachable_code)]
            return ::core::option::Option::Some(());
        }
    };
    types
        .iter()
        .map(|ty| {
            let typ = SolType::from_rust_type(ty).unwrap();
            let is_dynamic = typ.has_custom_types() || typ.is_dynamic() == Some(true);

            let decode = if !is_dynamic {
                quote! {
                    let __value = unsafe { <#ty as ::pvm_contract_sdk::StaticDecode>::decode_unchecked(&input, __decode_offset) };
                }
            } else {
                quote! {
                    let Ok(__value) = <#ty as ::pvm_contract_sdk::SolDecode>::decode_at(&input, __decode_offset) else {
                        <::pvm_contract_sdk::Host as ::pvm_contract_sdk::HostApi>::return_value(
                            this.host(),
                            ::pvm_contract_sdk::ReturnFlags::REVERT,
                            &::pvm_contract_sdk::framework_errors::INVALID_CALLDATA,
                        );
                        #ret
                    };
                }
            };

            quote! {{
                #decode
                __decode_offset += <#ty as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
                __value
            }}
        })
        .collect()
}

/// Build a compile-time expression for the minimum required input size.
pub fn calculate_min_input_size(types: &[syn::Type]) -> TokenStream {
    if types.is_empty() {
        return quote! { 0 };
    }
    let size_exprs: Vec<TokenStream> = types
        .iter()
        .map(|ty| {
            quote! { <#ty as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE }
        })
        .collect();
    quote! { 0 #(+ #size_exprs)* }
}
