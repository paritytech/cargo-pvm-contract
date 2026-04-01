use proc_macro2::TokenStream;
use quote::quote;

/// Generate an expression that encodes `value_expr` as ABI bytes
/// into a fixed-size stack buffer via `StaticEncodedLen`.
pub fn generate_encode(ty: &syn::Type, value_expr: TokenStream) -> TokenStream {
    quote! {{
        let mut __buf = [0u8; <#ty as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
        <#ty as ::pvm_contract_types::SolEncode>::encode_to(&#value_expr, &mut __buf);
        __buf
    }}
}
