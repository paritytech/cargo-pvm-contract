use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields};

use super::sol_type::{extract_field_info, sol_type_name_parts};
use crate::signature::SolType;

pub fn expand_sol_event(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let name_str = name.to_string();

    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        syn::Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolEvent can only be derived for structs",
            ));
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolEvent can only be derived for structs",
            ));
        }
    };

    let indexed_flags = collect_indexed_flags(fields)?;
    let field_info = extract_field_info(fields)?;

    let indexed_count = indexed_flags.iter().filter(|&&b| b).count();
    if indexed_count > 3 {
        return Err(syn::Error::new_spanned(
            name,
            "SolEvent supports at most 3 #[indexed] fields (EVM limit: 4 topics including topic0)",
        ));
    }

    validate_indexed_field_types(fields, &indexed_flags, &field_info)?;

    let sig_expr = build_signature_expr(&name_str, &field_info);
    let topic_expr = build_topic_expr(&name_str, &field_info);
    let indexed_count_lit = indexed_count;

    let topics_body = generate_topics_body(fields, &field_info, &indexed_flags);
    let data_body = generate_data_body(fields, &field_info, &indexed_flags);
    let abi_entry_expr = build_abi_entry_expr(&name_str, fields, &field_info, &indexed_flags);

    Ok(quote! {
        impl #name {
            #[doc(hidden)]
            pub const ABI_ENTRY: &'static str = #abi_entry_expr;
        }

        impl ::pvm_contract_types::SolEvent for #name {
            const TOPIC: [u8; 32] = #topic_expr;
            const NAME: &'static str = #name_str;
            const SIGNATURE: &'static str = #sig_expr;
            const INDEXED_COUNT: usize = #indexed_count_lit;

            fn topics(&self) -> alloc::vec::Vec<[u8; 32]> {
                #topics_body
            }

            fn data(&self) -> alloc::vec::Vec<u8> {
                #data_body
            }
        }

    })
}

fn build_abi_entry_expr(
    event_name: &str,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    indexed_flags: &[bool],
) -> TokenStream {
    let mut parts: Vec<TokenStream> = Vec::new();

    let header = format!(
        "{{\"type\":\"event\",\"name\":\"{}\",\"inputs\":[",
        event_name
    );
    parts.push(quote! { #header });

    if let Fields::Named(named) = fields {
        for (i, field) in named.named.iter().enumerate() {
            if i > 0 {
                parts.push(quote! { "," });
            }
            let field_name = field.ident.as_ref().unwrap().to_string();
            let prefix = format!("{{\"name\":\"{field_name}\",\"type\":\"");
            parts.push(quote! { #prefix });

            let sol_type = &field_info[i].1;
            if sol_type.has_custom_types() {
                let field_ty = &field.ty;
                parts.push(quote! {
                    <#field_ty as ::pvm_contract_types::SolEncode>::SOL_NAME
                });
            } else {
                let canonical = sol_type.canonical_name();
                parts.push(quote! { #canonical });
            }

            let suffix = if indexed_flags[i] {
                "\",\"indexed\":true}"
            } else {
                "\",\"indexed\":false}"
            };
            parts.push(quote! { #suffix });
        }
    }

    parts.push(quote! { "],\"anonymous\":false}" });

    quote! { ::pvm_contract_types::const_format::concatcp!(#(#parts),*) }
}

fn validate_indexed_field_types(
    fields: &Fields,
    indexed_flags: &[bool],
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<()> {
    let Fields::Named(named) = fields else {
        return Ok(());
    };

    for (i, field) in named.named.iter().enumerate() {
        if !indexed_flags[i] {
            continue;
        }
        let sol_type = &field_info[i].1;
        let unsupported = matches!(
            sol_type,
            SolType::Array(_) | SolType::FixedArray(_, _) | SolType::Tuple(_)
        );
        if unsupported {
            return Err(syn::Error::new_spanned(
                &field.ty,
                format!(
                    "SolEvent does not support `{}` as an indexed type. \
                     Arrays, fixed arrays, and tuples cannot be indexed.",
                    sol_type.canonical_name()
                ),
            ));
        }
    }
    Ok(())
}

fn collect_indexed_flags(fields: &Fields) -> syn::Result<Vec<bool>> {
    let mut flags = Vec::new();
    match fields {
        Fields::Named(named) => {
            for field in &named.named {
                let is_indexed = field
                    .attrs
                    .iter()
                    .any(|attr| attr.path().is_ident("indexed"));
                flags.push(is_indexed);
            }
        }
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                fields,
                "SolEvent requires named fields",
            ));
        }
        Fields::Unit => {}
    }
    Ok(flags)
}

fn build_signature_expr(
    event_name: &str,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let field_types: Vec<String> = field_info
            .iter()
            .map(|(_, sol_type)| sol_type.canonical_name())
            .collect();
        let sig = format!("{}({})", event_name, field_types.join(","));
        return quote! { #sig };
    }

    let mut parts: Vec<TokenStream> = Vec::new();
    let prefix = format!("{}(", event_name);
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

fn build_topic_expr(event_name: &str, field_info: &[(Option<syn::Ident>, SolType)]) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let field_types: Vec<String> = field_info
            .iter()
            .map(|(_, sol_type)| sol_type.canonical_name())
            .collect();
        let sig = format!("{}({})", event_name, field_types.join(","));
        let hash = pvm_contract_types::const_event_topic(&sig);
        let bytes = hash.iter().map(|b| quote! { #b });
        return quote! { [#(#bytes),*] };
    }

    let sig_expr = build_signature_expr(event_name, field_info);
    quote! { ::pvm_contract_types::const_event_topic(#sig_expr) }
}

fn generate_topics_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    indexed_flags: &[bool],
) -> TokenStream {
    let indexed_count = indexed_flags.iter().filter(|&&b| b).count();
    let capacity = indexed_count + 1;

    let mut topic_pushes = Vec::new();

    if let Fields::Named(named) = fields {
        for (i, field) in named.named.iter().enumerate() {
            if !indexed_flags[i] {
                continue;
            }
            let field_name = field.ident.as_ref().unwrap();
            let sol_type = &field_info[i].1;

            let pack = generate_indexed_topic_pack(field_name, sol_type, &field.ty);
            topic_pushes.push(pack);
        }
    }

    quote! {
        let mut __topics = alloc::vec::Vec::with_capacity(#capacity);
        __topics.push(Self::TOPIC);
        #(#topic_pushes)*
        __topics
    }
}

fn generate_indexed_topic_pack(
    field_name: &syn::Ident,
    _sol_type: &SolType,
    rust_type: &syn::Type,
) -> TokenStream {
    quote! {
        {
            const _: () = assert!(
                <#rust_type as ::pvm_contract_types::SolEncode>::IS_DYNAMIC
                    || <#rust_type as ::pvm_contract_types::SolEncode>::HEAD_SIZE <= 32,
                "SolEvent: #[indexed] static fields must fit in 32 bytes. \
                 Use the underlying primitive, or remove #[indexed]."
            );
            __topics.push(
                <#rust_type as ::pvm_contract_types::SolEncode>::indexed_topic(&self.#field_name)
            );
        }
    }
}

fn generate_data_body(
    fields: &Fields,
    _field_info: &[(Option<syn::Ident>, SolType)],
    indexed_flags: &[bool],
) -> TokenStream {
    let non_indexed: Vec<(usize, &syn::Ident)> = match fields {
        Fields::Named(named) => named
            .named
            .iter()
            .enumerate()
            .filter(|(i, _)| !indexed_flags[*i])
            .map(|(i, f)| (i, f.ident.as_ref().unwrap()))
            .collect(),
        _ => Vec::new(),
    };

    if non_indexed.is_empty() {
        return quote! { alloc::vec::Vec::new() };
    }

    let field_names: Vec<&syn::Ident> = non_indexed.iter().map(|(_, n)| *n).collect();
    let field_types: Vec<&syn::Type> = non_indexed
        .iter()
        .map(|&(i, _)| match fields {
            Fields::Named(named) => &named.named[i].ty,
            _ => unreachable!(),
        })
        .collect();

    if field_types.len() == 1 {
        let ft = field_types[0];
        let fn_ = field_names[0];
        return quote! {
            let __len = <#ft as ::pvm_contract_types::SolEncode>::encode_len(&self.#fn_);
            let mut __buf = alloc::vec![0u8; __len];
            <#ft as ::pvm_contract_types::SolEncode>::encode_to(&self.#fn_, &mut __buf);
            __buf
        };
    }

    let head_size_parts: Vec<TokenStream> = field_types
        .iter()
        .map(|ft| quote! { <#ft as ::pvm_contract_types::SolEncode>::SLOT_SIZE })
        .collect();

    let len_parts: Vec<TokenStream> = field_types
        .iter()
        .zip(field_names.iter())
        .map(|(ft, fn_)| {
            quote! {
                if <#ft as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                    <#ft as ::pvm_contract_types::SolEncode>::encode_body_len(&self.#fn_)
                } else {
                    0
                }
            }
        })
        .collect();

    let encode_stmts: Vec<TokenStream> = field_types
        .iter()
        .zip(field_names.iter())
        .map(|(ft, fn_)| {
            quote! {
                if <#ft as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                    __buf[__head_offset + 24..__head_offset + 32]
                        .copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                    let __body_len = <#ft as ::pvm_contract_types::SolEncode>::encode_body_len(&self.#fn_);
                    <#ft as ::pvm_contract_types::SolEncode>::encode_body_to(
                        &self.#fn_, &mut __buf[__tail_offset..__tail_offset + __body_len]);
                    __tail_offset += __body_len;
                } else {
                    <#ft as ::pvm_contract_types::SolEncode>::encode_body_to(
                        &self.#fn_, &mut __buf[__head_offset..__head_offset + 32]);
                }
                __head_offset += <#ft as ::pvm_contract_types::SolEncode>::SLOT_SIZE;
            }
        })
        .collect();

    quote! {
        let __head_size: usize = #(#head_size_parts)+*;
        let __total_len: usize = __head_size #(+ #len_parts)*;
        let mut __buf = alloc::vec![0u8; __total_len];
        let mut __head_offset: usize = 0;
        let mut __tail_offset: usize = __head_size;
        #(#encode_stmts)*
        __buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_enum() {
        let input: DeriveInput = syn::parse_str("enum Bad { A, B }").unwrap();
        let result = expand_sol_event(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("struct"), "Should reject enums: {err}");
    }

    #[test]
    fn rejects_more_than_three_indexed() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Bad {
                #[indexed] a: Address,
                #[indexed] b: Address,
                #[indexed] c: Address,
                #[indexed] d: Address,
            }"#,
        )
        .unwrap();
        let result = expand_sol_event(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("3"), "Should mention the limit: {err}");
    }

    #[test]
    fn accepts_basic_event() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Transfer {
                #[indexed] from: Address,
                #[indexed] to: Address,
                value: U256,
            }"#,
        )
        .unwrap();
        let result = expand_sol_event(input);
        assert!(result.is_ok(), "Should accept: {:?}", result.unwrap_err());
    }

    #[test]
    fn accepts_no_indexed_fields() {
        let input: DeriveInput = syn::parse_str("struct Log { value: u64 }").unwrap();
        let result = expand_sol_event(input);
        assert!(result.is_ok(), "Should accept: {:?}", result.unwrap_err());
    }

    #[test]
    fn accepts_all_indexed() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Approval {
                #[indexed] owner: Address,
                #[indexed] spender: Address,
                #[indexed] value: U256,
            }"#,
        )
        .unwrap();
        let result = expand_sol_event(input);
        assert!(result.is_ok(), "Should accept: {:?}", result.unwrap_err());
    }

    #[test]
    fn signature_for_known_types() {
        let fields = vec![
            (Some(syn::parse_str("from").unwrap()), SolType::Address),
            (Some(syn::parse_str("to").unwrap()), SolType::Address),
            (Some(syn::parse_str("value").unwrap()), SolType::Uint(256)),
        ];
        let sig = build_signature_expr("Transfer", &fields);
        let sig_str = sig.to_string();
        assert!(
            sig_str.contains("Transfer(address,address,uint256)"),
            "got: {sig_str}"
        );
    }

    #[test]
    fn topic_for_known_types_is_literal() {
        let fields = vec![
            (Some(syn::parse_str("from").unwrap()), SolType::Address),
            (Some(syn::parse_str("to").unwrap()), SolType::Address),
            (Some(syn::parse_str("value").unwrap()), SolType::Uint(256)),
        ];
        let topic = build_topic_expr("Transfer", &fields);
        let topic_str = topic.to_string();
        assert!(
            !topic_str.contains("const_event_topic"),
            "Known types should use literal topic: {topic_str}"
        );
    }

    #[test]
    fn rejects_indexed_dynamic_array() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Bad {
                #[indexed] items: Vec<u64>,
            }"#,
        )
        .unwrap();
        let err = expand_sol_event(input).unwrap_err().to_string();
        assert!(
            err.contains("indexed") && err.contains("uint64[]"),
            "Should reject Vec<_> indexed with a clear message: {err}"
        );
    }

    #[test]
    fn rejects_indexed_fixed_array() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Bad {
                #[indexed] items: [u64; 3],
            }"#,
        )
        .unwrap();
        let err = expand_sol_event(input).unwrap_err().to_string();
        assert!(
            err.contains("indexed") && err.contains("uint64[3]"),
            "Should reject fixed array indexed with a clear message: {err}"
        );
    }

    #[test]
    fn rejects_indexed_tuple() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Bad {
                #[indexed] pair: (u64, u64),
            }"#,
        )
        .unwrap();
        let err = expand_sol_event(input).unwrap_err().to_string();
        assert!(
            err.contains("indexed") && err.contains("(uint64,uint64)"),
            "Should reject tuple indexed with a clear message: {err}"
        );
    }

    // Custom-typed indexed fields pass derive-time validation because `Custom`
    // also covers type aliases like `type Owner = Address;`. Incompatible
    // custom types (dynamic, or composite > 32 bytes) are rejected by a
    // `const _: () = assert!(...)` in the generated code.
    #[test]
    fn accepts_indexed_custom_type_at_derive_time() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ownership {
                #[indexed] inner: MyAlias,
                value: U256,
            }"#,
        )
        .unwrap();
        assert!(expand_sol_event(input).is_ok());
    }

    #[test]
    fn topic_for_custom_types_uses_const_event_topic() {
        let fields = vec![(
            Some(syn::parse_str("data").unwrap()),
            SolType::Custom("MyStruct".to_string()),
        )];
        let topic = build_topic_expr("MyEvent", &fields);
        let topic_str = topic.to_string();
        assert!(
            topic_str.contains("const_event_topic"),
            "Custom types should use const_event_topic: {topic_str}"
        );
    }
}
