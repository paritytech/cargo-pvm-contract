use proc_macro2::TokenStream;
use quote::quote;
use syn::{DataEnum, DeriveInput, Fields, FieldsUnnamed, Type};

use super::sol_type::{
    build_dynamic_head_size_expr, extract_field_info, generate_dynamic_encode_body,
    generate_dynamic_encode_len, sol_type_name_parts,
};
use crate::{codegen::sol_type::generate_dynamic_decode_body, signature::SolType};

pub fn expand_sol_error_enum(input: &DeriveInput, data: &DataEnum) -> syn::Result<TokenStream> {
    if !data.variants.iter().all(|x| {
        matches!(&x.fields, Fields::Unnamed(FieldsUnnamed { unnamed, .. }) if unnamed.len() == 1)
    }) || data.variants.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "SolError can only be derived for enums that contain unnamed fields with path to a struct that implements SolError",
        ));
    }
    struct Arm {
        size: TokenStream,
        encode: TokenStream,
        decode: TokenStream,
        from: TokenStream,
        ty: Type,
    }
    let name = &input.ident;

    let res: Vec<Arm> = data
        .variants
        .iter()
        .filter_map(|x| match &x.fields {
            Fields::Unnamed(FieldsUnnamed { unnamed, .. }) if unnamed.len() == 1 => {
                Some((x.ident.clone(), unnamed.first().unwrap()))
            }
            _ => None,
        })
        .map(|(variant_name, field)| {
            let ty = &field.ty;
            Arm {
                ty: ty.clone(),
                size: quote! {
                    Self::#variant_name(i) => i.encoded_size()
                },
                encode: quote! {
                    Self::#variant_name(i) => i.encode_to(buf)
                },
                decode: quote! {
                    if let Some(res) = #ty::decode_at(input, offset)? {
                        return Ok(Some(Self::#variant_name(res)));
                    }
                },
                from: quote! {
                    impl From<#ty> for #name {
                        fn from(value: #ty) -> #name {
                            #name::#variant_name(value)
                        }
                    }
                },
            }
        })
        .collect();
    let decoders = res.iter().map(|x| &x.decode);
    let encoders = res.iter().map(|x| &x.encode);
    let size = res.iter().map(|x| &x.size);
    let from = res.iter().map(|x| &x.from);
    let tys = res.iter().map(|x| &x.ty);

    Ok(quote! {

        #(#from)*

        impl ::pvm_contract_sdk::SolError for #name {
            const SIGNATURE: &'static str = "";

            fn encoded_size(&self) -> usize {
                match self {
                   #(#size),*
                }
            }

            fn encode_to(&self, buf: &mut [u8]) -> usize {
                match self {
                    #(#encoders),*
                }
            }

            fn decode_at(input: &[u8], offset: usize) -> Result<Option<Self>, ::pvm_contract_sdk::DecodeError> {
                #(#decoders)*

                Ok(None)
            }

            #[cfg(feature = "abi-gen")]
            fn error_signatures() -> impl Iterator<Item = &'static &'static str>
            where
                Self: Sized,
            {
                let mut arr = [];
                let arr = arr.into_iter();
                let arr = arr #(.chain(<#tys as ::pvm_contract_sdk::SolError>::error_signatures()))*;
                arr.into_iter()
            }
        }
    })
}

pub fn expand_sol_error(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let name_str = name.to_string();

    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        syn::Data::Enum(data) => return expand_sol_error_enum(&input, data),
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
    let decode = generate_dynamic_decode_body(fields, &field_info);

    Ok(quote! {
        impl ::pvm_contract_sdk::SolError for #name {
            const SELECTOR: [u8; 4] = #selector_expr;
            const SIGNATURE: &'static str = #sig_expr;

            fn encoded_size(&self) -> usize {
                #encoded_size_body
            }

            fn encode_to(&self, buf: &mut [u8]) -> usize {
                buf[0..4].copy_from_slice(&Self::SELECTOR);
                let mut buf = &mut buf[4..];
                let size = {
                    #encode_body
                };
                4 + size
            }

            fn decode_at(input: &[u8], offset: usize) -> Result<Option<Self>, ::pvm_contract_sdk::DecodeError> {
                if input.len() < 4 {
                    return Err(::pvm_contract_sdk::DecodeError);
                };
                if input
                    .get(offset..offset + 4)
                    .is_some_and(|x| x == Self::SELECTOR)
                {
                    let input = &input[4..];
                    let res = {
                        #decode
                    }?;
                    Ok(Some(res))
                } else {
                    Ok(None)
                }
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
    quote! { ::pvm_contract_sdk::const_format::concatcp!(#(#parts),*) }
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
    quote! { ::pvm_contract_sdk::const_selector(#sig_expr) }
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

    #[test]
    fn enum_accept() {
        use syn::parse::{Parse, Parser};

        let input: DeriveInput = syn::parse_str(
            "
        enum Err {
            Err1(Err1),
            Err2(Err2)
        }
        ",
        )
        .unwrap();
        let result = expand_sol_error(input).unwrap();
        let file = prettyplease::unparse(&syn::File::parse.parse2(result).unwrap());
        expect_test::expect![[r#"
            impl From<Err1> for Err {
                fn from(value: Err1) -> Err {
                    Err::Err1(value)
                }
            }
            impl From<Err2> for Err {
                fn from(value: Err2) -> Err {
                    Err::Err2(value)
                }
            }
            impl ::pvm_contract_sdk::SolError for Err {
                const SIGNATURE: &'static str = "";
                fn encoded_size(&self) -> usize {
                    match self {
                        Self::Err1(i) => i.encoded_size(),
                        Self::Err2(i) => i.encoded_size(),
                    }
                }
                fn encode_to(&self, buf: &mut [u8]) -> usize {
                    match self {
                        Self::Err1(i) => i.encode_to(buf),
                        Self::Err2(i) => i.encode_to(buf),
                    }
                }
                fn decode_at(
                    input: &[u8],
                    offset: usize,
                ) -> Result<Option<Self>, ::pvm_contract_sdk::DecodeError> {
                    if let Some(res) = Err1::decode_at(input, offset)? {
                        return Ok(Some(Self::Err1(res)));
                    }
                    if let Some(res) = Err2::decode_at(input, offset)? {
                        return Ok(Some(Self::Err2(res)));
                    }
                    Ok(None)
                }
                #[cfg(feature = "abi-gen")]
                fn error_signatures() -> impl Iterator<Item = &'static &'static str>
                where
                    Self: Sized,
                {
                    let mut arr = [];
                    let arr = arr.into_iter();
                    let arr = arr
                        .chain(<Err1 as ::pvm_contract_sdk::SolError>::error_signatures())
                        .chain(<Err2 as ::pvm_contract_sdk::SolError>::error_signatures());
                    arr.into_iter()
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn no_op_struct() {
        use syn::parse::{Parse, Parser};

        let input: DeriveInput = syn::parse_str(
            "
        struct Err;
        ",
        )
        .unwrap();
        let result = expand_sol_error(input).unwrap();
        let file = prettyplease::unparse(&syn::File::parse.parse2(result).unwrap());
        expect_test::expect![[r#"
            impl ::pvm_contract_sdk::SolError for Err {
                const SELECTOR: [u8; 4] = [198u8, 79u8, 195u8, 114u8];
                const SIGNATURE: &'static str = "Err()";
                fn encoded_size(&self) -> usize {
                    4
                }
                fn encode_to(&self, buf: &mut [u8]) -> usize {
                    buf[0..4].copy_from_slice(&Self::SELECTOR);
                    let mut buf = &mut buf[4..];
                    let size = { 0 };
                    4 + size
                }
                fn decode_at(
                    input: &[u8],
                    offset: usize,
                ) -> Result<Option<Self>, ::pvm_contract_sdk::DecodeError> {
                    if input.len() < 4 {
                        return Err(::pvm_contract_sdk::DecodeError);
                    }
                    if input.get(offset..offset + 4).is_some_and(|x| x == Self::SELECTOR) {
                        let input = &input[4..];
                        let res = { Ok(Self) }?;
                        Ok(Some(res))
                    } else {
                        Ok(None)
                    }
                }
            }
        "#]]
        .assert_eq(&file);
    }
}
