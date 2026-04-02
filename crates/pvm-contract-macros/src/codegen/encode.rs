use proc_macro2::TokenStream;
use quote::quote;

/// Generate an expression that encodes `value_expr` as ABI bytes
/// into a fixed-size stack buffer via `StaticEncodedLen`.
///
/// Only valid for static types. Dynamic types (String, Vec, Bytes)
/// require alloc mode — the const assert produces a clear error.
pub fn generate_encode(ty: &syn::Type, value_expr: TokenStream) -> TokenStream {
    quote! {{
        const { assert!(
            !<#ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC,
            "dynamic types (String, Vec, Bytes) require allocator = \"pico\" or \"bump\""
        ) };
        let mut __buf = [0u8; <#ty as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
        <#ty as ::pvm_contract_types::SolEncode>::encode_to(&#value_expr, &mut __buf);
        __buf
    }}
}
