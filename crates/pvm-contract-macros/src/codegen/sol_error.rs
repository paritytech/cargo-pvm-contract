use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields};

use super::sol_type::{
    build_dynamic_head_size_expr, extract_field_info, generate_dynamic_encode_body,
    generate_dynamic_encode_len, sol_type_name_parts,
};
use crate::signature::SolType;

pub fn expand_sol_error(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let name_str = name.to_string();

    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        syn::Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolError can only be derived for structs. Use sol_revert_enum! for error enums.",
            ));
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolError cannot be derived for unions",
            ));
        }
    };

    let field_info = extract_field_info(fields)?;

    // Build SIGNATURE and SELECTOR
    let sig_expr = build_signature_expr(&name_str, &field_info);
    let selector_expr = build_selector_expr(&name_str, &field_info);

    let (encode_body, encoded_size_body) = generate_error_encode(fields, &field_info);

    Ok(quote! {
        impl ::pvm_contract_types::SolError for #name {
            const SELECTOR: [u8; 4] = #selector_expr;
            const SIGNATURE: &'static str = #sig_expr;

            fn encode_params(&self, buf: &mut [u8]) -> usize {
                #encode_body
            }

            fn encoded_size(&self) -> usize {
                #encoded_size_body
            }
        }
    })
}

fn generate_error_encode(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> (TokenStream, TokenStream) {
    if field_info.is_empty() {
        return (quote! { 0 }, quote! { 4 });
    }

    let head_size_expr = build_dynamic_head_size_expr(fields, field_info);
    let encode_len_body = generate_dynamic_encode_len(fields, field_info, &head_size_expr);
    // generate_dynamic_encode_body declares `let mut __tail_offset = head_size;`
    // and advances it as it encodes fields (works for both static and dynamic)
    let encode_body = generate_dynamic_encode_body(fields, field_info, &head_size_expr);

    let encoded_size = quote! { 4 + #encode_len_body };
    let encode = quote! {
        #encode_body
        __tail_offset
    };

    (encode, encoded_size)
}

fn build_signature_expr(
    error_name: &str,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let field_types: Vec<String> = field_info
            .iter()
            .map(|(_, sol_type)| sol_type.canonical_name())
            .collect();
        let sig = format!("{}({})", error_name, field_types.join(","));
        return quote! { #sig };
    }

    let mut parts: Vec<TokenStream> = Vec::new();
    let prefix = format!("{}(", error_name);
    parts.push(quote! { #prefix });

    for (i, (_, sol_type)) in field_info.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        sol_type_name_parts(sol_type, &mut parts);
    }

    parts.push(quote! { ")" });
    quote! { ::pvm_contract_types::const_format::concatcp!(#(#parts),*) }
}

fn build_selector_expr(
    error_name: &str,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let field_types: Vec<String> = field_info
            .iter()
            .map(|(_, sol_type)| sol_type.canonical_name())
            .collect();
        let sig = format!("{}({})", error_name, field_types.join(","));
        let selector = crate::signature::compute_selector(&sig);
        let [s0, s1, s2, s3] = selector;
        return quote! { [#s0, #s1, #s2, #s3] };
    }

    let sig_expr = build_signature_expr(error_name, field_info);
    quote! { ::pvm_contract_types::const_selector(#sig_expr) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_signature_for_known_types() {
        let fields = vec![
            (Some(syn::parse_str("account").unwrap()), SolType::Address),
            (
                Some(syn::parse_str("required").unwrap()),
                SolType::Uint(256),
            ),
        ];
        let sig = build_signature_expr("InsufficientBalance", &fields);
        let sig_str = sig.to_string();
        assert!(
            sig_str.contains("InsufficientBalance(address,uint256)"),
            "got: {sig_str}"
        );
    }

    #[test]
    fn build_signature_for_custom_types_uses_concatcp() {
        let fields = vec![(
            Some(syn::parse_str("point").unwrap()),
            SolType::Custom("Point".to_string()),
        )];
        let sig = build_signature_expr("MyError", &fields);
        let sig_str = sig.to_string();
        assert!(
            sig_str.contains("concatcp"),
            "Custom types should use concatcp: {sig_str}"
        );
        assert!(
            sig_str.contains("SOL_NAME"),
            "Custom types should reference SOL_NAME: {sig_str}"
        );
    }

    #[test]
    fn build_selector_for_known_types_is_literal() {
        let fields = vec![(Some(syn::parse_str("x").unwrap()), SolType::Uint(64))];
        let sel = build_selector_expr("Foo", &fields);
        let sel_str = sel.to_string();
        assert!(
            !sel_str.contains("const_selector"),
            "Known types should use literal selector: {sel_str}"
        );
    }

    #[test]
    fn build_selector_for_custom_types_uses_const_selector() {
        let fields = vec![(
            Some(syn::parse_str("p").unwrap()),
            SolType::Custom("Point".to_string()),
        )];
        let sel = build_selector_expr("MyError", &fields);
        let sel_str = sel.to_string();
        assert!(
            sel_str.contains("const_selector"),
            "Custom types should use const_selector: {sel_str}"
        );
    }

    #[test]
    fn rejects_enum() {
        let input: DeriveInput = syn::parse_str("enum Bad { A, B }").unwrap();
        let result = expand_sol_error(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("enum"), "Should reject enums: {err}");
    }

    #[test]
    fn accepts_unit_struct() {
        let input: DeriveInput = syn::parse_str("struct Empty;").unwrap();
        let result = expand_sol_error(input);
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_static_fields() {
        let input: DeriveInput =
            syn::parse_str("struct GoodError { account: Address, amount: U256 }").unwrap();
        let result = expand_sol_error(input);
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_dynamic_fields() {
        let input: DeriveInput = syn::parse_str("struct DynError { msg: String }").unwrap();
        let result = expand_sol_error(input);
        assert!(result.is_ok(), "Dynamic fields should be accepted now");
    }
}
