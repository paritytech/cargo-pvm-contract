use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields, Type};

use crate::signature::SolType;

pub fn expand_sol_type(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;

    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        syn::Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolType can only be derived for structs",
            ));
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolType can only be derived for structs",
            ));
        }
    };

    let field_info = extract_field_info(fields)?;

    if field_info.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "SolType requires at least one field",
        ));
    }

    // Unresolved custom types cannot be queried via SolType::is_dynamic; route
    // through dynamic codegen, which now uses trait-based runtime/static checks.
    let has_dynamic = field_info
        .iter()
        .any(|(_, t)| t.has_custom_types() || t.is_dynamic() == Some(true));
    if has_dynamic {
        expand_dynamic_sol_type(name, fields, &field_info)
    } else {
        expand_static_sol_type(name, fields, &field_info)
    }
}

fn expand_static_sol_type(
    name: &syn::Ident,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<TokenStream> {
    let sol_name_expr = build_sol_name_expr(field_info);
    let total_size_expr = build_total_size_expr(field_info);
    let encode_body = generate_static_encode_body(fields);
    let decode_body = generate_static_decode_body(fields);

    #[cfg(feature = "abi-gen")]
    let abi_param_fn = generate_abi_param_fn(fields, field_info);
    #[cfg(not(feature = "abi-gen"))]
    let abi_param_fn = quote::quote! {};

    Ok(quote! {
        impl ::pvm_contract_types::SolEncode for #name {
            const IS_DYNAMIC: bool = false;
            const SOL_NAME: &'static str = #sol_name_expr;
            const HEAD_SIZE: usize = #total_size_expr;

            #[inline]
            fn encode_body_len(&self) -> usize {
                #total_size_expr
            }

            fn encode_body_to(&self, buf: &mut [u8]) {
                #encode_body
            }

            #abi_param_fn
        }

        impl ::pvm_contract_types::StaticEncodedLen for #name {
            const ENCODED_SIZE: usize = #total_size_expr;
        }

        impl ::pvm_contract_types::SolDecode for #name {
            fn decode_at(input: &[u8], offset: usize) -> Self {
                #decode_body
            }
        }

        impl ::pvm_contract_types::SolArrayElement for #name {}
    })
}

fn expand_dynamic_sol_type(
    name: &syn::Ident,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<TokenStream> {
    let sol_name_expr = build_sol_name_expr(field_info);
    let is_dynamic_expr = build_is_dynamic_expr(fields, field_info);
    let head_size_expr = build_dynamic_head_size_expr(fields, field_info);
    let encode_len_body = generate_dynamic_encode_len(fields, field_info, &head_size_expr);
    let encode_body = generate_dynamic_encode_body(fields, field_info, &head_size_expr);
    let decode_body = generate_dynamic_decode_body(fields, field_info);

    #[cfg(feature = "abi-gen")]
    let abi_param_fn = generate_abi_param_fn(fields, field_info);
    #[cfg(not(feature = "abi-gen"))]
    let abi_param_fn = quote::quote! {};

    Ok(quote! {
        impl ::pvm_contract_types::SolEncode for #name {
            const IS_DYNAMIC: bool = #is_dynamic_expr;
            const SOL_NAME: &'static str = #sol_name_expr;
            const HEAD_SIZE: usize = #head_size_expr;

            fn encode_body_len(&self) -> usize {
                #encode_len_body
            }

            fn encode_body_to(&self, buf: &mut [u8]) {
                #encode_body
            }

            #abi_param_fn
        }

        impl ::pvm_contract_types::SolDecode for #name {
            fn decode_at(input: &[u8], offset: usize) -> Self {
                #decode_body
            }

            fn decode_tail(input: &[u8], offset: usize) -> Self {
                Self::decode_at(input, offset)
            }
        }

        impl ::pvm_contract_types::SolArrayElement for #name {}
    })
}

/// Generate the `abi_param()` method override for a struct.
/// Returns `"type": "tuple"` with `components` listing each field.
#[cfg(feature = "abi-gen")]
fn generate_abi_param_fn(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let field_types = get_field_types(fields);

    let component_exprs: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .map(|((field_name, _), field_ty)| {
            let name_str = match field_name {
                Some(ident) => ident.to_string(),
                None => String::new(),
            };
            quote! {
                <#field_ty as ::pvm_contract_types::SolEncode>::abi_param(#name_str)
            }
        })
        .collect();

    quote! {
        fn abi_param(name: &str) -> ::pvm_contract_types::AbiParam {
            extern crate alloc;
            ::pvm_contract_types::AbiParam {
                name: alloc::string::String::from(name),
                param_type: alloc::string::String::from("tuple"),
                components: alloc::vec![#(#component_exprs),*],
            }
        }
    }
}

fn build_is_dynamic_expr(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());
    if !has_custom {
        let is_dynamic = field_info.iter().any(|(_, t)| t.is_dynamic() == Some(true));
        return quote! { #is_dynamic };
    }

    let field_types = get_field_types(fields);
    let parts: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .map(|((_, t), ty)| match t.is_dynamic() {
            Some(is_dyn) => quote! { #is_dyn },
            None => quote! { <#ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC },
        })
        .collect();

    quote! { false #(|| #parts)* }
}

pub(crate) fn sol_type_name_parts(ty: &SolType, parts: &mut Vec<TokenStream>) {
    match ty {
        SolType::Custom(name) => match syn::parse_str::<syn::Path>(name) {
            Ok(type_path) => {
                parts.push(quote! { <#type_path as ::pvm_contract_types::SolEncode>::SOL_NAME });
            }
            Err(err) => {
                let msg =
                    format!("Invalid custom type path `{name}` in `#[derive(SolType)]`: {err}");
                parts.push(quote! { compile_error!(#msg) });
            }
        },
        SolType::Array(inner) if inner.has_custom_types() => {
            sol_type_name_parts(inner, parts);
            parts.push(quote! { "[]" });
        }
        SolType::FixedArray(inner, size) if inner.has_custom_types() => {
            sol_type_name_parts(inner, parts);
            let suffix = format!("[{}]", size);
            parts.push(quote! { #suffix });
        }
        SolType::Tuple(types) if types.iter().any(|t| t.has_custom_types()) => {
            parts.push(quote! { "(" });
            for (i, t) in types.iter().enumerate() {
                if i > 0 {
                    parts.push(quote! { "," });
                }
                sol_type_name_parts(t, parts);
            }
            parts.push(quote! { ")" });
        }
        _ => {
            let name = ty.canonical_name();
            parts.push(quote! { #name });
        }
    }
}

fn build_sol_signature(field_info: &[(Option<syn::Ident>, SolType)]) -> String {
    let field_types = field_info
        .iter()
        .map(|(_, sol_type)| sol_type.canonical_name())
        .collect::<Vec<_>>();
    format!("({})", field_types.join(","))
}

fn build_sol_name_expr(field_info: &[(Option<syn::Ident>, SolType)]) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let sig = build_sol_signature(field_info);
        return quote! { #sig };
    }

    let mut parts: Vec<TokenStream> = Vec::new();
    parts.push(quote! { "(" });

    for (i, (_, sol_type)) in field_info.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        sol_type_name_parts(sol_type, &mut parts);
    }

    parts.push(quote! { ")" });
    quote! { ::pvm_contract_types::const_format::concatcp!(#(#parts),*) }
}

fn build_total_size_expr(field_info: &[(Option<syn::Ident>, SolType)]) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());

    if !has_custom {
        let total: usize = field_info
            .iter()
            .map(|(_, t)| {
                t.head_size()
                    .expect("build_total_size_expr called on unresolved custom type")
            })
            .sum();
        return quote! { #total };
    }

    let size_exprs: Vec<TokenStream> = field_info
        .iter()
        .map(|(_, sol_type)| sol_type_head_size_expr(sol_type))
        .collect();

    quote! { 0 #(+ #size_exprs)* }
}

fn sol_type_head_size_expr(ty: &SolType) -> TokenStream {
    match ty {
        SolType::Custom(name) => match syn::parse_str::<syn::Path>(name) {
            Ok(type_path) => {
                quote! { <#type_path as ::pvm_contract_types::SolEncode>::HEAD_SIZE }
            }
            Err(err) => {
                let msg =
                    format!("Invalid custom type path `{name}` in `#[derive(SolType)]`: {err}");
                quote! {{
                    compile_error!(#msg);
                    0usize
                }}
            }
        },
        SolType::FixedArray(inner, size) if inner.has_custom_types() => {
            let inner_size = sol_type_head_size_expr(inner);
            let size_lit = *size;
            quote! { (#inner_size * #size_lit) }
        }
        SolType::Tuple(types) if types.iter().any(|t| t.has_custom_types()) => {
            let parts: Vec<TokenStream> = types.iter().map(sol_type_head_size_expr).collect();
            quote! { (0 #(+ #parts)*) }
        }
        _ => {
            let size = ty
                .head_size()
                .expect("sol_type_head_size_expr called on unresolved custom type");
            quote! { #size }
        }
    }
}

fn get_field_types(fields: &Fields) -> Vec<&Type> {
    match fields {
        Fields::Named(named) => named.named.iter().map(|f| &f.ty).collect(),
        Fields::Unnamed(unnamed) => unnamed.unnamed.iter().map(|f| &f.ty).collect(),
        Fields::Unit => vec![],
    }
}

// -----------------------------------------------------------------------
// Static struct encode/decode — always uses trait-based dispatch
// -----------------------------------------------------------------------

fn generate_static_encode_body(fields: &Fields) -> TokenStream {
    let field_types = get_field_types(fields);
    let mut stmts = Vec::new();
    stmts.push(quote! { let mut __offset: usize = 0; });

    for (i, field_ty) in field_types.iter().enumerate() {
        let field_access = match fields {
            Fields::Named(named) => {
                let name = named.named[i].ident.as_ref().unwrap();
                quote! { self.#name }
            }
            Fields::Unnamed(_) => {
                let idx = syn::Index::from(i);
                quote! { self.#idx }
            }
            Fields::Unit => continue,
        };

        stmts.push(quote! {
            let __hs = <#field_ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE;
            ::pvm_contract_types::SolEncode::encode_body_to(&#field_access, &mut buf[__offset..__offset + __hs]);
            __offset += __hs;
        });
    }

    quote! { #(#stmts)* }
}

fn generate_static_decode_body(fields: &Fields) -> TokenStream {
    match fields {
        Fields::Named(named) => {
            let mut pre_stmts: Vec<TokenStream> = vec![quote! { let mut __offset: usize = 0; }];
            let mut field_lets = Vec::new();

            for field in &named.named {
                let name = field.ident.as_ref().unwrap();
                let ty = &field.ty;
                let tmp = quote::format_ident!("__field_{}", name);

                pre_stmts.push(quote! {
                    let #tmp = {
                        let __val = <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __offset);
                        __offset += <#ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE;
                        __val
                    };
                });
                field_lets.push(quote! { #name: #tmp });
            }

            quote! {
                #(#pre_stmts)*
                Self { #(#field_lets),* }
            }
        }
        Fields::Unnamed(unnamed) => {
            let mut pre_stmts: Vec<TokenStream> = vec![quote! { let mut __offset: usize = 0; }];
            let mut field_tmps = Vec::new();

            for (i, field) in unnamed.unnamed.iter().enumerate() {
                let ty = &field.ty;
                let tmp = quote::format_ident!("__field_{}", i);

                pre_stmts.push(quote! {
                    let #tmp = {
                        let __val = <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __offset);
                        __offset += <#ty as ::pvm_contract_types::SolEncode>::HEAD_SIZE;
                        __val
                    };
                });
                field_tmps.push(quote! { #tmp });
            }

            quote! {
                #(#pre_stmts)*
                Self(#(#field_tmps),*)
            }
        }
        Fields::Unit => quote! { Self },
    }
}

// -----------------------------------------------------------------------
// Dynamic struct helpers
// -----------------------------------------------------------------------

/// Compute the total head size expression for a dynamic struct.
pub(crate) fn build_dynamic_head_size_expr(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let field_types = get_field_types(fields);
    build_dynamic_head_sum_expr(field_info, &field_types)
}

/// Compute the head offset expression for field at position `idx` in a dynamic struct.
fn build_dynamic_field_offset_expr(
    field_info: &[(Option<syn::Ident>, SolType)],
    field_types: &[&Type],
    idx: usize,
) -> TokenStream {
    build_dynamic_head_sum_expr(&field_info[..idx], &field_types[..idx])
}

/// Build a sum expression of dynamic-head contributions for a field slice.
///
/// For known non-custom fields we constant-fold to a literal. When custom types are present,
/// we use trait metadata (`IS_DYNAMIC` and `HEAD_SIZE`) to avoid guessing dynamic/static shape.
fn build_dynamic_head_sum_expr(
    field_info: &[(Option<syn::Ident>, SolType)],
    field_types: &[&Type],
) -> TokenStream {
    let has_custom = field_info.iter().any(|(_, t)| t.has_custom_types());
    if !has_custom {
        let total: usize = field_info
            .iter()
            .map(|(_, t)| {
                if t.is_dynamic() == Some(true) {
                    32
                } else {
                    t.head_size()
                        .expect("build_dynamic_head_sum_expr called on unresolved custom type")
                }
            })
            .sum();
        return quote! { #total };
    }

    let parts: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .map(|((_, t), ty)| match t.is_dynamic() {
            Some(true) => quote! { 32usize },
            Some(false) => {
                let size = t.head_size().unwrap();
                quote! { #size }
            }
            None => quote! {
                <#ty as ::pvm_contract_types::SolEncode>::SLOT_SIZE
            },
        })
        .collect();

    quote! { (0 #(+ #parts)*) }
}

pub(crate) fn generate_dynamic_encode_len(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size_expr: &TokenStream,
) -> TokenStream {
    let field_types = get_field_types(fields);
    let tail_lens: Vec<TokenStream> = field_info
        .iter()
        .zip(field_types.iter())
        .enumerate()
        .filter_map(|(i, ((field_name, sol_type), field_ty))| {
            let field_access = match fields {
                Fields::Named(_) => {
                    let name = field_name.as_ref().unwrap();
                    quote! { self.#name }
                }
                Fields::Unnamed(_) => {
                    let idx = syn::Index::from(i);
                    quote! { self.#idx }
                }
                Fields::Unit => return None,
            };

            match sol_type.is_dynamic() {
                Some(true) => Some(quote! {
                    ::pvm_contract_types::SolEncode::encode_body_len(&#field_access)
                }),
                Some(false) => None,
                None => Some(quote! {
                    if <#field_ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                        ::pvm_contract_types::SolEncode::encode_body_len(&#field_access)
                    } else {
                        0usize
                    }
                }),
            }
        })
        .collect();

    quote! {
        #head_size_expr #(+ #tail_lens)*
    }
}

pub(crate) fn generate_dynamic_encode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size_expr: &TokenStream,
) -> TokenStream {
    let field_types = get_field_types(fields);
    let mut stmts = Vec::new();

    for (i, (field_name, sol_type)) in field_info.iter().enumerate() {
        let field_access = match fields {
            Fields::Named(_) => {
                let name = field_name.as_ref().unwrap();
                quote! { self.#name }
            }
            Fields::Unnamed(_) => {
                let idx = syn::Index::from(i);
                quote! { self.#idx }
            }
            Fields::Unit => continue,
        };

        let head_offset_expr = build_dynamic_field_offset_expr(field_info, &field_types, i);
        let field_ty = field_types[i];

        match sol_type.is_dynamic() {
            Some(true) => {
                stmts.push(quote! {
                    {
                        let __ho = #head_offset_expr;
                        buf[__ho..__ho + 24].fill(0);
                        buf[__ho + 24..__ho + 32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                        let __tail_len = ::pvm_contract_types::SolEncode::encode_body_len(&#field_access);
                        ::pvm_contract_types::SolEncode::encode_body_to(&#field_access, &mut buf[__tail_offset..__tail_offset + __tail_len]);
                        __tail_offset += __tail_len;
                    }
                });
            }
            Some(false) => {
                stmts.push(quote! {
                    {
                        let __ho = #head_offset_expr;
                        ::pvm_contract_types::SolEncode::encode_body_to(&#field_access, &mut buf[__ho..]);
                    }
                });
            }
            None => {
                stmts.push(quote! {
                    {
                        let __ho = #head_offset_expr;
                        if <#field_ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                            buf[__ho..__ho + 24].fill(0);
                            buf[__ho + 24..__ho + 32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                            let __tail_len = ::pvm_contract_types::SolEncode::encode_body_len(&#field_access);
                            ::pvm_contract_types::SolEncode::encode_body_to(&#field_access, &mut buf[__tail_offset..__tail_offset + __tail_len]);
                            __tail_offset += __tail_len;
                        } else {
                            ::pvm_contract_types::SolEncode::encode_body_to(&#field_access, &mut buf[__ho..]);
                        }
                    }
                });
            }
        }
    }

    quote! {
        let mut __tail_offset: usize = #head_size_expr;
        #(#stmts)*
    }
}

fn generate_dynamic_decode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let field_types = get_field_types(fields);
    match fields {
        Fields::Named(named) => {
            let field_decodes: Vec<_> = named
                .named
                .iter()
                .zip(field_info.iter())
                .enumerate()
                .map(|(i, (field, (field_name, sol_type)))| {
                    let name = field_name.as_ref().unwrap();
                    let ty = &field.ty;
                    let head_offset_expr =
                        build_dynamic_field_offset_expr(field_info, &field_types, i);
                    let decode = generate_dynamic_field_decode(ty, sol_type, &head_offset_expr);
                    quote! {
                        #name: #decode
                    }
                })
                .collect();

            quote! {
                Self { #(#field_decodes),* }
            }
        }
        Fields::Unnamed(unnamed) => {
            let field_decodes: Vec<_> = unnamed
                .unnamed
                .iter()
                .zip(field_info.iter())
                .enumerate()
                .map(|(i, (field, (_, sol_type)))| {
                    let ty = &field.ty;
                    let head_offset_expr =
                        build_dynamic_field_offset_expr(field_info, &field_types, i);
                    generate_dynamic_field_decode(ty, sol_type, &head_offset_expr)
                })
                .collect();

            quote! {
                Self(#(#field_decodes),*)
            }
        }
        Fields::Unit => quote! { Self },
    }
}

fn generate_dynamic_field_decode(
    ty: &Type,
    sol_type: &SolType,
    head_offset_expr: &TokenStream,
) -> TokenStream {
    match sol_type.is_dynamic() {
        Some(true) => quote! {{
            let __ho = #head_offset_expr;
            let __field_offset =
                u64::from_be_bytes(input[offset + __ho + 24..offset + __ho + 32].try_into().unwrap())
                    as usize;
            <#ty as ::pvm_contract_types::SolDecode>::decode_tail(input, offset + __field_offset)
        }},
        Some(false) => quote! {{
            let __ho = #head_offset_expr;
            <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __ho)
        }},
        None => quote! {{
            let __ho = #head_offset_expr;
            if <#ty as ::pvm_contract_types::SolEncode>::IS_DYNAMIC {
                let __field_offset =
                    u64::from_be_bytes(input[offset + __ho + 24..offset + __ho + 32].try_into().unwrap())
                        as usize;
                <#ty as ::pvm_contract_types::SolDecode>::decode_tail(input, offset + __field_offset)
            } else {
                <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __ho)
            }
        }},
    }
}

pub(crate) fn extract_field_info(
    fields: &Fields,
) -> syn::Result<Vec<(Option<syn::Ident>, SolType)>> {
    let mut result = Vec::new();

    match fields {
        Fields::Named(named) => {
            for field in &named.named {
                let sol_type = type_to_sol_type(&field.ty)?;
                result.push((field.ident.clone(), sol_type));
            }
        }
        Fields::Unnamed(unnamed) => {
            for field in &unnamed.unnamed {
                let sol_type = type_to_sol_type(&field.ty)?;
                result.push((None, sol_type));
            }
        }
        Fields::Unit => {}
    }

    Ok(result)
}

fn type_to_sol_type(ty: &Type) -> syn::Result<SolType> {
    SolType::from_rust_type(ty).ok_or_else(|| {
        syn::Error::new_spanned(
            ty,
            "Unsupported type for SolType derive. Supported types: \
                 U256, u128, u64, u32, u16, u8, i128, i64, i32, i16, i8, \
                 bool, [u8; 20] (address), [u8; N] (bytesN), String. \
                 For custom structs, derive SolType on them first."
                .to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::SolType;

    fn normalize_tokens(ts: TokenStream) -> String {
        ts.to_string().split_whitespace().collect::<String>()
    }

    #[test]
    fn custom_type_field_total_size_uses_trait_expression() {
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("x").unwrap()),
                SolType::Uint(64),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("count").unwrap()),
                SolType::Custom("Count".to_string()),
            ),
        ];
        let expr = build_total_size_expr(&field_info);
        let expected =
            quote! { 0 + 32usize + <Count as ::pvm_contract_types::SolEncode>::HEAD_SIZE };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn build_sol_name_expr_uses_concatcp_for_custom_types() {
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![(
            Some(syn::parse_str::<syn::Ident>("count").unwrap()),
            SolType::Custom("Count".to_string()),
        )];
        let expr = build_sol_name_expr(&field_info);
        let expected = quote! {
            ::pvm_contract_types::const_format::concatcp!(
                "(",
                <Count as ::pvm_contract_types::SolEncode>::SOL_NAME,
                ")"
            )
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn build_sol_name_expr_uses_literal_for_known_types() {
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("x").unwrap()),
                SolType::Uint(64),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("y").unwrap()),
                SolType::Uint(64),
            ),
        ];
        let expr = build_sol_name_expr(&field_info);
        let expected = quote! { "(uint64,uint64)" };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn dynamic_head_size_uses_trait_dynamic_for_custom_types() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct S {
                a: Count,
                b: u64,
            }
        };

        let fields = match &input.data {
            syn::Data::Struct(data) => &data.fields,
            _ => panic!("expected struct"),
        };

        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("a").unwrap()),
                SolType::Custom("Count".to_string()),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("b").unwrap()),
                SolType::Uint(64),
            ),
        ];

        let expr = build_dynamic_head_size_expr(fields, &field_info);
        let expected = quote! {
            (0 +
                <Count as ::pvm_contract_types::SolEncode>::SLOT_SIZE
                + 32usize)
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn dynamic_field_offset_uses_trait_dynamic_for_custom_prefix() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct S {
                a: Count,
                b: u64,
                c: bool,
            }
        };

        let fields = match &input.data {
            syn::Data::Struct(data) => &data.fields,
            _ => panic!("expected struct"),
        };

        let field_types = get_field_types(fields);
        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("a").unwrap()),
                SolType::Custom("Count".to_string()),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("b").unwrap()),
                SolType::Uint(64),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("c").unwrap()),
                SolType::Bool,
            ),
        ];

        let expr = build_dynamic_field_offset_expr(&field_info, &field_types, 2);
        let expected = quote! {
            (0 +
                <Count as ::pvm_contract_types::SolEncode>::SLOT_SIZE
                + 32usize)
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn build_is_dynamic_expr_uses_trait_for_custom_types() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct S {
                point: Point,
                value: u64,
            }
        };

        let fields = match &input.data {
            syn::Data::Struct(data) => &data.fields,
            _ => panic!("expected struct"),
        };

        let field_info: Vec<(Option<syn::Ident>, SolType)> = vec![
            (
                Some(syn::parse_str::<syn::Ident>("point").unwrap()),
                SolType::Custom("Point".to_string()),
            ),
            (
                Some(syn::parse_str::<syn::Ident>("value").unwrap()),
                SolType::Uint(64),
            ),
        ];

        let expr = build_is_dynamic_expr(fields, &field_info);
        let expected = quote! {
            false || <Point as ::pvm_contract_types::SolEncode>::IS_DYNAMIC || false
        };
        assert_eq!(normalize_tokens(expr), normalize_tokens(expected));
    }

    #[test]
    fn expand_sol_type_does_not_force_true_is_dynamic_for_static_custom_struct() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct Line {
                a: Point,
                b: Point,
            }
        };

        let expanded = normalize_tokens(expand_sol_type(input).unwrap());
        let expected_is_dynamic = normalize_tokens(quote! {
            const IS_DYNAMIC: bool = false
                || <Point as ::pvm_contract_types::SolEncode>::IS_DYNAMIC
                || <Point as ::pvm_contract_types::SolEncode>::IS_DYNAMIC;
        });
        assert!(expanded.contains(&expected_is_dynamic));
    }

    #[test]
    fn expand_sol_type_keeps_known_dynamic_fields_in_is_dynamic_expr() {
        let input: syn::DeriveInput = syn::parse_quote! {
            struct NamedPoint {
                point: Point,
                name: alloc::string::String,
            }
        };

        let expanded = normalize_tokens(expand_sol_type(input).unwrap());
        let expected_is_dynamic = normalize_tokens(quote! {
            const IS_DYNAMIC: bool = false
                || <Point as ::pvm_contract_types::SolEncode>::IS_DYNAMIC
                || true;
        });
        assert!(expanded.contains(&expected_is_dynamic));
    }
}
