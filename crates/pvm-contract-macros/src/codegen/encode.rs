use proc_macro2::TokenStream;
use quote::quote;

/// Generate an expression that encodes `value_expr` as ABI bytes.
///
/// - **alloc mode**: heap-allocated `Vec<u8>` via `encode_len` + `encode_to`
/// - **stack mode**: fixed-size `[u8; ENCODED_SIZE]` via `StaticEncodedLen`
pub fn generate_encode(ty: &syn::Type, value_expr: TokenStream, use_alloc: bool) -> TokenStream {
    if use_alloc {
        quote! {{
            let __len = <#ty as ::pvm_contract_types::SolEncode>::encode_len(&#value_expr);
            let mut __buf = alloc::vec![0u8; __len];
            <#ty as ::pvm_contract_types::SolEncode>::encode_to(&#value_expr, &mut __buf);
            __buf
        }}
    } else {
        quote! {{
            let mut __buf = [0u8; <#ty as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
            <#ty as ::pvm_contract_types::SolEncode>::encode_to(&#value_expr, &mut __buf);
            __buf
        }}
    }
}
