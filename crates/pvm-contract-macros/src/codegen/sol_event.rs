use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields};

use super::sol_type::{extract_field_info, sol_type_name_parts};
use crate::signature::SolType;

/// Expand `#[derive(SolEvent)]` into a `SolEvent` trait impl.
///
/// No allocator required. Topics use a stack-allocated `EventTopics` struct
/// (max 4 entries). Data encoding writes into a caller-provided buffer via
/// `data_to(&self, buf: &mut [u8])`.
///
/// Indexed field handling:
/// - Static primitives (address, uintN, bool, bytesN): encoded directly into the topic slot.
/// - Dynamic primitives (string, bytes): `keccak256(raw_bytes)`.
/// - Static arrays, fixed arrays, tuples: `keccak256(abi.encode(value))`.
/// - Dynamic composites (tuples/arrays with dynamic elements): rejected at compile time.
/// - Dynamic arrays (`Vec<T>`): rejected at compile time.
/// - Custom/alias types: rejected at compile time.
///
/// For events with all-static non-indexed fields, an `emit(host)` convenience
/// method is generated automatically. For dynamic events, add `#[alloc]` to
/// generate an alloc-backed `emit()`, or use `data_len()` + `data_to()` manually.
///
/// Anonymous events are supported via `#[anonymous]` on the struct. Anonymous
/// events skip topic[0] (the signature hash) and allow up to 4 indexed fields.
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

    let mut is_anonymous = false;
    let mut use_alloc = false;
    for attr in &input.attrs {
        if attr.path().is_ident("anonymous") {
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[anonymous] takes no arguments",
                ));
            }
            is_anonymous = true;
        }
        if attr.path().is_ident("alloc") {
            if !matches!(attr.meta, syn::Meta::Path(_)) {
                return Err(syn::Error::new_spanned(attr, "#[alloc] takes no arguments"));
            }
            use_alloc = true;
        }
    }

    let indexed_count = indexed_flags.iter().filter(|&&b| b).count();
    let max_indexed = if is_anonymous { 4 } else { 3 };
    if indexed_count > max_indexed {
        return Err(syn::Error::new_spanned(
            name,
            format!(
                "SolEvent supports at most {} #[indexed] fields{}",
                max_indexed,
                if is_anonymous {
                    " for anonymous events"
                } else {
                    ""
                }
            ),
        ));
    }

    // Reject #[indexed] on dynamic arrays (`Vec<T>`). The proc macro cannot
    // statically size a runtime-variable encode buffer in no-alloc mode.
    // Custom struct types are allowed: SolType derive overrides
    // `indexed_topic()` to do `keccak256(abi.encode(self))`, and primitive
    // aliases (e.g. `type Owner = Address`) fall through to the default
    // right-aligned `indexed_topic()` which is also correct.
    if let Fields::Named(named) = fields {
        for (i, field) in named.named.iter().enumerate() {
            if !indexed_flags[i] {
                continue;
            }
            if let SolType::Array(_) = &field_info[i].1 {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "#[indexed] does not support dynamic arrays (Vec<T>); \
                     use a fixed-size array instead",
                ));
            }
        }
    }

    let sig_expr = build_signature_expr(&name_str, &field_info);
    let topic_expr = build_topic_expr(&name_str, &field_info);
    let indexed_count_lit = indexed_count;

    let topics_body = generate_topics_body(fields, &field_info, &indexed_flags, is_anonymous);
    let (data_len_body, data_body) = generate_data_bodies(fields, &field_info, &indexed_flags);
    let abi_item_body = build_abi_item_body(&name_str, fields, &indexed_flags, is_anonymous);

    // Check if all non-indexed fields are static (compile-time known size).
    // Only generate emit() for static events; dynamic events require the
    // user to manage the data buffer via data_len() + data_to().
    let non_indexed_info: Vec<(&syn::Type, &SolType)> = match fields {
        Fields::Named(named) => named
            .named
            .iter()
            .enumerate()
            .filter(|(i, _)| !indexed_flags[*i])
            .map(|(i, f)| (&f.ty as &syn::Type, &field_info[i].1))
            .collect(),
        _ => Vec::new(),
    };
    let all_non_indexed_static = non_indexed_info
        .iter()
        .all(|(_, sol_type)| !sol_type.is_dynamic().unwrap_or(true));

    let emit_method = if all_non_indexed_static {
        // Static data: stack-allocated buffer sized at compile time.
        let data_size_parts: Vec<TokenStream> = non_indexed_info
            .iter()
            .map(|(ft, _)| quote! { <#ft as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE })
            .collect();
        let data_size_expr = if data_size_parts.is_empty() {
            quote! { 0 }
        } else {
            quote! { #(#data_size_parts)+* }
        };
        quote! {
            /// Emit this event via the host.
            pub fn emit(&self, host: &::pvm_contract_sdk::Host) {
                use ::pvm_contract_sdk::SolEvent as _;
                use ::pvm_contract_sdk::HostApi as _;
                let __topics = self.topics();
                // Safe to use HEAD_SIZE sum: all non-indexed fields are static.
                let mut __data = [0u8; #data_size_expr];
                self.data_to(&mut __data);
                host.deposit_event(&__topics, &__data);
            }
        }
    } else if use_alloc {
        // Dynamic data with #[alloc]: heap-allocated buffer sized at runtime.
        quote! {
            /// Emit this event via the host. Uses alloc for the data buffer
            /// because this event has dynamic non-indexed fields.
            pub fn emit(&self, host: &::pvm_contract_sdk::Host) {
                extern crate alloc;
                use ::pvm_contract_sdk::SolEvent as _;
                use ::pvm_contract_sdk::HostApi as _;
                let __topics = self.topics();
                let __len = self.data_len();
                let mut __data = alloc::vec![0u8; __len];
                self.data_to(&mut __data);
                host.deposit_event(&__topics, &__data);
            }
        }
    } else {
        // Dynamic event without `#[alloc]`: no `emit()` is generated. Callers
        // use `data_len()` + `data_to()` manually, or add `#[alloc]`.
        quote! {}
    };

    Ok(quote! {
        impl #name {
            /// Build the `AbiItem::Event` description used by the ABI JSON
            /// generator. Available when the `abi-gen` feature is enabled.
            #[cfg(feature = "abi-gen")]
            #[doc(hidden)]
            pub fn abi_item() -> ::pvm_contract_sdk::AbiItem {
                #abi_item_body
            }

            #emit_method
        }

        impl ::pvm_contract_sdk::SolEvent for #name {
            const TOPIC: [u8; 32] = #topic_expr;
            const NAME: &'static str = #name_str;
            const SIGNATURE: &'static str = #sig_expr;
            const INDEXED_COUNT: usize = #indexed_count_lit;

            fn topics(&self) -> ::pvm_contract_sdk::EventTopics {
                #topics_body
            }

            fn data_len(&self) -> usize {
                #data_len_body
            }

            fn data_to(&self, __buf: &mut [u8]) {
                #data_body
            }
        }

    })
}

/// Generate the body of `abi_item()`: returns `AbiItem::Event { ... }` with
/// each input built from the field's `SolEncode::abi_param` (which expands
/// tuples into `type:"tuple"` with nested `components`).
fn build_abi_item_body(
    event_name: &str,
    fields: &Fields,
    indexed_flags: &[bool],
    is_anonymous: bool,
) -> TokenStream {
    let mut input_exprs: Vec<TokenStream> = Vec::new();

    if let Fields::Named(named) = fields {
        for (i, field) in named.named.iter().enumerate() {
            let field_name = field.ident.as_ref().unwrap().to_string();
            let field_ty = &field.ty;
            let indexed = indexed_flags[i];
            input_exprs.push(quote! {
                {
                    let __p = <#field_ty as ::pvm_contract_sdk::SolEncode>::abi_param(#field_name);
                    ::pvm_contract_sdk::AbiEventParam {
                        name: __p.name,
                        param_type: __p.param_type,
                        indexed: #indexed,
                        components: __p.components,
                    }
                }
            });
        }
    }

    // Use `alloc` (not `std`) for consistency with the emit() path. `abi_item()`
    // is gated on `feature = "abi-gen"`, where `alloc` is always available.
    quote! {
        {
            extern crate alloc;
            ::pvm_contract_sdk::AbiItem::Event {
                name: alloc::string::String::from(#event_name),
                inputs: alloc::vec![#(#input_exprs),*],
                anonymous: #is_anonymous,
            }
        }
    }
}

fn collect_indexed_flags(fields: &Fields) -> syn::Result<Vec<bool>> {
    let mut flags = Vec::new();
    match fields {
        Fields::Named(named) => {
            for field in &named.named {
                let mut is_indexed = false;
                for attr in &field.attrs {
                    if !attr.path().is_ident("indexed") {
                        continue;
                    }
                    if !matches!(attr.meta, syn::Meta::Path(_)) {
                        return Err(syn::Error::new_spanned(
                            attr,
                            "#[indexed] takes no arguments",
                        ));
                    }
                    is_indexed = true;
                }
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
    quote! { ::pvm_contract_sdk::const_format::concatcp!(#(#parts),*) }
}

fn build_topic_expr(event_name: &str, field_info: &[(Option<syn::Ident>, SolType)]) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let field_types: Vec<String> = field_info
            .iter()
            .map(|(_, sol_type)| sol_type.canonical_name())
            .collect();
        let sig = format!("{}({})", event_name, field_types.join(","));
        let hash = pvm_contract_types::const_keccak256(sig.as_bytes());
        let bytes = hash.iter().map(|b| quote! { #b });
        return quote! { [#(#bytes),*] };
    }

    let sig_expr = build_signature_expr(event_name, field_info);
    quote! { ::pvm_contract_sdk::const_keccak256((#sig_expr).as_bytes()) }
}

fn generate_topics_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    indexed_flags: &[bool],
    is_anonymous: bool,
) -> TokenStream {
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
        let mut __topics = ::pvm_contract_sdk::EventTopics::new();
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
    // Arrays, fixed arrays, and tuples are encoded as `keccak256(abi.encode(value))`
    // per the Solidity event spec — done inline here because the proc macro
    // can identify them syntactically and the trait's default `indexed_topic`
    // is the right-aligned primitive form.
    let needs_abi_encode_hash = matches!(
        sol_type,
        SolType::Array(_) | SolType::FixedArray(_, _) | SolType::Tuple(_)
    );

    if needs_abi_encode_hash {
        // Stack buffer sized by HEAD_SIZE — only correct for static composites.
        // Dynamic composites (tuples/arrays with dynamic elements) are caught
        // by the IS_DYNAMIC assertion.
        quote! {
            {
                const _: () = assert!(
                    !<#rust_type as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC,
                    "SolEvent: #[indexed] composites (tuples, fixed arrays) must \
                     be fully static. Dynamic composites require runtime-sized \
                     buffers which are not supported in no-alloc mode."
                );
                const __ENC_SIZE: usize = <#rust_type as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE;
                let mut __enc_buf = [0u8; __ENC_SIZE];
                <#rust_type as ::pvm_contract_sdk::SolEncode>::encode_to(&self.#field_name, &mut __enc_buf);
                __topics.push(::pvm_contract_sdk::keccak256(&__enc_buf));
            }
        }
    } else if matches!(sol_type, SolType::Custom(_)) {
        // Custom types include both primitive aliases (`type Owner = Address`)
        // and `#[derive(SolType)]` structs. The proc macro can't distinguish
        // the two, so we dispatch via the `indexed_topic` trait method:
        // - Primitive aliases inherit the default right-aligned encoding.
        // - `SolType` derive overrides `indexed_topic` to do
        //   `keccak256(abi.encode(self))`, matching Solidity for structs.
        // Indexed dynamic composites are rejected at compile time.
        quote! {
            {
                const _: () = assert!(
                    !<#rust_type as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC,
                    "SolEvent: #[indexed] custom types must be static. \
                     Dynamic composites are not supported."
                );
                __topics.push(
                    <#rust_type as ::pvm_contract_sdk::SolEncode>::indexed_topic(&self.#field_name)
                );
            }
        }
    } else {
        // Built-in primitives (uintN, intN, bool, address, bytesN) and
        // dynamic primitives (string, bytes) — `indexed_topic` defaults are
        // correct. The HEAD_SIZE assertion guards against a manual SolEncode
        // impl that would silently truncate or panic on a value > 32 bytes.
        quote! {
            {
                const _: () = assert!(
                    <#rust_type as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC
                        || <#rust_type as ::pvm_contract_sdk::SolEncode>::HEAD_SIZE <= 32,
                    "SolEvent: #[indexed] static fields must fit in 32 bytes. \
                     Use the underlying primitive, or remove #[indexed]."
                );
                __topics.push(
                    <#rust_type as ::pvm_contract_sdk::SolEncode>::indexed_topic(&self.#field_name)
                );
            }
        }
    }
}

/// Generate both `data_len()` and `data_to()` bodies.
fn generate_data_bodies(
    fields: &Fields,
    _field_info: &[(Option<syn::Ident>, SolType)],
    indexed_flags: &[bool],
) -> (TokenStream, TokenStream) {
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
        return (quote! { 0 }, quote! {});
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
        let len_body = quote! {
            <#ft as ::pvm_contract_sdk::SolEncode>::encode_len(&self.#fn_)
        };
        let to_body = quote! {
            <#ft as ::pvm_contract_sdk::SolEncode>::encode_to(&self.#fn_, __buf);
        };
        return (len_body, to_body);
    }

    let head_size_parts: Vec<TokenStream> = field_types
        .iter()
        .map(|ft| quote! { <#ft as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE })
        .collect();

    let len_parts: Vec<TokenStream> = field_types
        .iter()
        .zip(field_names.iter())
        .map(|(ft, fn_)| {
            quote! {
                if <#ft as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
                    <#ft as ::pvm_contract_sdk::SolEncode>::encode_body_len(&self.#fn_)
                } else {
                    0
                }
            }
        })
        .collect();

    let len_body = quote! {
        let __head_size: usize = #(#head_size_parts)+*;
        __head_size #(+ #len_parts)*
    };

    let encode_stmts: Vec<TokenStream> = field_types
        .iter()
        .zip(field_names.iter())
        .map(|(ft, fn_)| {
            quote! {
                if <#ft as ::pvm_contract_sdk::SolEncode>::IS_DYNAMIC {
                    __buf[__head_offset + 24..__head_offset + 32]
                        .copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                    let __body_len = <#ft as ::pvm_contract_sdk::SolEncode>::encode_body_len(&self.#fn_);
                    <#ft as ::pvm_contract_sdk::SolEncode>::encode_body_to(
                        &self.#fn_, &mut __buf[__tail_offset..__tail_offset + __body_len]);
                    __tail_offset += __body_len;
                } else {
                    let __slot = <#ft as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
                    <#ft as ::pvm_contract_sdk::SolEncode>::encode_body_to(
                        &self.#fn_, &mut __buf[__head_offset..__head_offset + __slot]);
                }
                __head_offset += <#ft as ::pvm_contract_sdk::SolEncode>::SLOT_SIZE;
            }
        })
        .collect();

    let to_body = quote! {
        let __head_size: usize = #(#head_size_parts)+*;
        let mut __head_offset: usize = 0;
        let mut __tail_offset: usize = __head_size;
        #(#encode_stmts)*
    };

    (len_body, to_body)
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
            !topic_str.contains("const_keccak256"),
            "Known types should use literal topic: {topic_str}"
        );
    }

    #[test]
    fn rejects_indexed_dynamic_array() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ev {
                #[indexed] items: Vec<u64>,
            }"#,
        )
        .unwrap();
        let err = expand_sol_event(input).unwrap_err().to_string();
        assert!(
            err.contains("dynamic arrays"),
            "should reject Vec<T> as indexed: {err}"
        );
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

    // Custom types (`#[derive(SolType)]` structs and type aliases) are now
    // accepted as indexed fields. SolType derive overrides `indexed_topic`
    // to produce `keccak256(abi.encode(self))`, and primitive aliases fall
    // through to the default right-aligned encoding.
    #[test]
    fn accepts_indexed_custom_type() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ownership {
                #[indexed] inner: MyType,
                value: U256,
            }"#,
        )
        .unwrap();
        assert!(expand_sol_event(input).is_ok());
    }

    #[test]
    fn topic_for_custom_types_uses_const_keccak256() {
        let fields = vec![(
            Some(syn::parse_str("data").unwrap()),
            SolType::Custom("MyStruct".to_string()),
        )];
        let topic = build_topic_expr("MyEvent", &fields);
        let topic_str = topic.to_string();
        assert!(
            topic_str.contains("const_keccak256"),
            "Custom types should use const_keccak256: {topic_str}"
        );
    }

    #[test]
    fn emit_not_generated_for_dynamic_non_indexed_fields() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Log {
                #[indexed] who: Address,
                message: String,
            }"#,
        )
        .unwrap();
        let output = expand_sol_event(input).unwrap().to_string();
        assert!(
            !output.contains("fn emit"),
            "emit() should not be generated for events with dynamic non-indexed fields without #[alloc]"
        );
    }

    #[test]
    fn indexed_composite_generates_dynamic_assert() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Ev {
                #[indexed] pair: (u64, u64),
                value: U256,
            }"#,
        )
        .unwrap();
        let output = expand_sol_event(input).unwrap().to_string();
        assert!(
            output.contains("IS_DYNAMIC"),
            "indexed composites should have an IS_DYNAMIC const_assert: {output}"
        );
    }

    #[test]
    fn emit_generated_for_static_non_indexed_fields() {
        let input: DeriveInput = syn::parse_str(
            r#"struct Transfer {
                #[indexed] from: Address,
                #[indexed] to: Address,
                value: U256,
            }"#,
        )
        .unwrap();
        let output = expand_sol_event(input).unwrap().to_string();
        assert!(
            output.contains("fn emit"),
            "emit() should be generated for events with static non-indexed fields"
        );
    }

    #[test]
    fn alloc_attr_generates_emit_for_dynamic_fields() {
        let input: DeriveInput = syn::parse_str(
            r#"
            #[alloc]
            struct Log {
                #[indexed] who: Address,
                message: String,
            }"#,
        )
        .unwrap();
        let output = expand_sol_event(input).unwrap().to_string();
        assert!(
            output.contains("fn emit"),
            "emit() should be generated with #[alloc] for dynamic events"
        );
    }

    #[test]
    fn alloc_attr_with_args_rejected() {
        let input: DeriveInput = syn::parse_str(
            r#"
            #[alloc(true)]
            struct Log {
                message: String,
            }"#,
        )
        .unwrap();
        let err = expand_sol_event(input).unwrap_err().to_string();
        assert!(
            err.contains("no arguments"),
            "should reject #[alloc(true)]: {err}"
        );
    }
}
