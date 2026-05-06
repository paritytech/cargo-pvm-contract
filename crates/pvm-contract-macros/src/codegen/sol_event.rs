use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields};

use super::sol_type::{extract_field_info, sol_type_name_parts};
use crate::signature::SolType;

/// Expand `#[derive(SolEvent)]` into a `SolEvent` trait impl.
///
/// Requires an allocator (`topics()` and `data()` return `Vec`).
/// Inside `#[contract]` modules with an allocator, `alloc` is already in scope.
/// For standalone use, the generated code emits `extern crate alloc` in each method body.
///
/// Indexed field handling:
/// - Static primitives (address, uintN, bool, bytesN): encoded directly into the topic slot.
/// - Dynamic primitives (string, bytes): `keccak256(raw_bytes)`.
/// - Arrays, fixed arrays, tuples: `keccak256(abi.encode(value))`.
/// - Custom/alias types: rejected at compile time. Use the concrete
///   Solidity-mapped type directly (e.g. `Address` instead of `type Owner = Address`).
///
/// Anonymous events are supported via `#[anonymous]` on the struct. Anonymous
/// events skip topic\[0\] (the signature hash) and allow up to 4 indexed fields.
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

    let is_anonymous = {
        let mut found = false;
        for attr in &input.attrs {
            if attr.path().is_ident("anonymous") {
                if !matches!(attr.meta, syn::Meta::Path(_)) {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "#[anonymous] takes no arguments",
                    ));
                }
                found = true;
            }
        }
        found
    };

    let indexed_count = indexed_flags.iter().filter(|&&b| b).count();
    let max_indexed = if is_anonymous { 4 } else { 3 };
    if indexed_count > max_indexed {
        return Err(syn::Error::new_spanned(
            name,
            format!(
                "SolEvent supports at most {} #[indexed] fields{}",
                max_indexed,
                if is_anonymous {
                    " (anonymous event: 4 topics)"
                } else {
                    " (EVM limit: 4 topics including topic0)"
                }
            ),
        ));
    }

    // Reject #[indexed] on custom/alias types. The proc macro cannot
    // distinguish type aliases (type Owner = Address) from actual custom
    // structs (#[derive(SolType)]). For aliases, indexed_topic() would
    // produce correct output, but for custom structs it produces topics
    // incompatible with Solidity (direct encoding instead of
    // keccak256(abi.encode(value))), and dynamic custom structs panic at
    // runtime. Reject all Custom types to guarantee correctness.
    if let Fields::Named(named) = fields {
        for (i, field) in named.named.iter().enumerate() {
            if !indexed_flags[i] {
                continue;
            }
            if matches!(field_info[i].1, SolType::Custom(_)) {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "#[indexed] does not support custom/alias types; \
                     use the concrete Solidity-mapped type directly",
                ));
            }
        }
    }

    let sig_expr = build_signature_expr(&name_str, &field_info);
    let topic_expr = build_topic_expr(&name_str, &field_info);
    let indexed_count_lit = indexed_count;

    let topics_body = generate_topics_body(fields, &field_info, &indexed_flags, is_anonymous);
    let data_body = generate_data_body(fields, &field_info, &indexed_flags);
    let abi_entry_expr =
        build_abi_entry_expr(&name_str, fields, &field_info, &indexed_flags, is_anonymous);

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
                extern crate alloc;
                #topics_body
            }

            fn data(&self) -> alloc::vec::Vec<u8> {
                extern crate alloc;
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
    is_anonymous: bool,
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

    let anon_str = if is_anonymous {
        "],\"anonymous\":true}"
    } else {
        "],\"anonymous\":false}"
    };
    parts.push(quote! { #anon_str });

    quote! { ::pvm_contract_types::const_format::concatcp!(#(#parts),*) }
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
    is_anonymous: bool,
) -> TokenStream {
    let indexed_count = indexed_flags.iter().filter(|&&b| b).count();
    let capacity = if is_anonymous {
        indexed_count
    } else {
        indexed_count + 1
    };

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

    let topic0_push = if is_anonymous {
        quote! {}
    } else {
        quote! { __topics.push(Self::TOPIC); }
    };

    quote! {
        let mut __topics = alloc::vec::Vec::with_capacity(#capacity);
        #topic0_push
        #(#topic_pushes)*
        __topics
    }
}

fn generate_indexed_topic_pack(
    field_name: &syn::Ident,
    sol_type: &SolType,
    rust_type: &syn::Type,
) -> TokenStream {
    // Arrays, fixed arrays, and tuples use keccak256(abi.encode(value)) per Solidity spec.
    // Custom types are rejected at validation time, so only primitives reach the else branch.
    let needs_abi_encode_hash = matches!(
        sol_type,
        SolType::Array(_) | SolType::FixedArray(_, _) | SolType::Tuple(_)
    );

    if needs_abi_encode_hash {
        // Solidity hashes indexed reference types via keccak256(abi.encode(value)).
        // This handles arrays, fixed arrays, and tuples.
        quote! {
            {
                extern crate alloc;
                let __enc_len = <#rust_type as ::pvm_contract_types::SolEncode>::encode_len(&self.#field_name);
                let mut __enc_buf = alloc::vec![0u8; __enc_len];
                <#rust_type as ::pvm_contract_types::SolEncode>::encode_to(&self.#field_name, &mut __enc_buf);
                __topics.push(::pvm_contract_types::keccak256(&__enc_buf));
            }
        }
    } else {
        // Static and dynamic primitives use indexed_topic() directly.
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
    fn anonymous_allows_four_indexed() {
        let input: DeriveInput = syn::parse_str(
            r#"
            #[anonymous]
            struct Anon {
                #[indexed] a: Address,
                #[indexed] b: Address,
                #[indexed] c: Address,
                #[indexed] d: Address,
            }"#,
        )
        .unwrap();
        assert!(expand_sol_event(input).is_ok());
    }

    #[test]
    fn anonymous_rejects_five_indexed() {
        let input: DeriveInput = syn::parse_str(
            r#"
            #[anonymous]
            struct Anon {
                #[indexed] a: Address,
                #[indexed] b: Address,
                #[indexed] c: Address,
                #[indexed] d: Address,
                #[indexed] e: Address,
            }"#,
        )
        .unwrap();
        let err = expand_sol_event(input).unwrap_err().to_string();
        assert!(err.contains("4"), "Should mention the limit of 4: {err}");
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
    fn accepts_indexed_dynamic_array() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ev {
                #[indexed] items: Vec<u64>,
            }"#,
        )
        .unwrap();
        assert!(expand_sol_event(input).is_ok());
    }

    #[test]
    fn accepts_indexed_fixed_array() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ev {
                #[indexed] items: [u64; 3],
            }"#,
        )
        .unwrap();
        assert!(expand_sol_event(input).is_ok());
    }

    #[test]
    fn accepts_indexed_tuple() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ev {
                #[indexed] pair: (u64, u64),
            }"#,
        )
        .unwrap();
        assert!(expand_sol_event(input).is_ok());
    }

    // Custom/alias types are rejected as indexed fields. The proc macro
    // cannot distinguish type aliases from actual custom structs, so all
    // Custom types are rejected to guarantee correctness.
    #[test]
    fn rejects_indexed_custom_type() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ownership {
                #[indexed] inner: MyAlias,
                value: U256,
            }"#,
        )
        .unwrap();
        let err = expand_sol_event(input).unwrap_err().to_string();
        assert!(
            err.contains("custom/alias"),
            "should reject indexed custom types: {err}"
        );
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
