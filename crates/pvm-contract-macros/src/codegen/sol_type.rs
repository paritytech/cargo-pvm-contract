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

    let has_dynamic = field_info.iter().any(|(_, t)| t.is_dynamic());
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
    let sol_name = build_sol_signature(field_info);
    let total_size: usize = field_info.iter().map(|(_, t)| t.head_size()).sum();
    let encode_body = generate_static_encode_body(fields, field_info);
    let decode_body = generate_static_decode_body(fields, field_info);

    Ok(quote! {
        impl ::pvm_contract_types::SolEncode for #name {
            const IS_DYNAMIC: bool = false;

            #[inline]
            fn encode_len(&self) -> usize {
                #total_size
            }

            fn encode_to(&self, buf: &mut [u8]) {
                #encode_body
            }

            #[cfg(feature = "abi-reflection")]
            fn sol_name() -> ::alloc::string::String {
                ::alloc::string::String::from(#sol_name)
            }
        }

        impl ::pvm_contract_types::StaticEncodedLen for #name {
            const ENCODED_SIZE: usize = #total_size;
        }

        impl ::pvm_contract_types::SolDecode for #name {
            fn decode_at(input: &[u8], offset: usize) -> Self {
                #decode_body
            }
        }
    })
}

fn expand_dynamic_sol_type(
    name: &syn::Ident,
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> syn::Result<TokenStream> {
    let sol_name = build_sol_signature(field_info);
    let head_size: usize = field_info.len() * 32;
    let encode_len_body = generate_dynamic_encode_len(fields, field_info, head_size);
    let encode_body = generate_dynamic_encode_body(fields, field_info, head_size);
    let decode_body = generate_dynamic_decode_body(fields, field_info);

    Ok(quote! {
        impl ::pvm_contract_types::SolEncode for #name {
            const IS_DYNAMIC: bool = true;

            fn encode_len(&self) -> usize {
                #encode_len_body
            }

            fn encode_to(&self, buf: &mut [u8]) {
                #encode_body
            }

            #[cfg(feature = "abi-reflection")]
            fn sol_name() -> ::alloc::string::String {
                ::alloc::string::String::from(#sol_name)
            }
        }

        impl ::pvm_contract_types::SolDecode for #name {
            fn decode_at(input: &[u8], offset: usize) -> Self {
                #decode_body
            }

            fn decode_tail(input: &[u8], offset: usize) -> Self {
                Self::decode_at(input, offset)
            }
        }
    })
}

fn build_sol_signature(field_info: &[(Option<syn::Ident>, SolType)]) -> String {
    let field_types = field_info
        .iter()
        .map(|(_, sol_type)| sol_type.canonical_name())
        .collect::<Vec<_>>();
    format!("({})", field_types.join(","))
}

fn generate_dynamic_encode_len(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size: usize,
) -> TokenStream {
    let tail_lens: Vec<TokenStream> = field_info
        .iter()
        .enumerate()
        .filter_map(|(i, (field_name, sol_type))| {
            if !sol_type.is_dynamic() {
                return None;
            }
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
            Some(quote! {
                ::pvm_contract_types::SolEncode::tail_len(&#field_access)
            })
        })
        .collect();

    quote! {
        #head_size #(+ #tail_lens)*
    }
}

fn generate_dynamic_encode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size: usize,
) -> TokenStream {
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

        let head_offset = i * 32;

        if sol_type.is_dynamic() {
            stmts.push(quote! {
                buf[#head_offset..#head_offset + 24].fill(0);
                buf[#head_offset + 24..#head_offset + 32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                let __tail_len = ::pvm_contract_types::SolEncode::tail_len(&#field_access);
                ::pvm_contract_types::SolEncode::encode_tail_to(&#field_access, &mut buf[__tail_offset..__tail_offset + __tail_len]);
                __tail_offset += __tail_len;
            });
        } else {
            let encode_stmt = generate_field_encode(sol_type, &field_access, head_offset);
            stmts.push(encode_stmt);
        }
    }

    quote! {
        let mut __tail_offset: usize = #head_size;
        #(#stmts)*
    }
}

fn extract_field_info(fields: &Fields) -> syn::Result<Vec<(Option<syn::Ident>, SolType)>> {
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

fn generate_static_encode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let mut offset = 0usize;
    let mut encode_stmts = Vec::new();

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

        let encode_stmt = generate_field_encode(sol_type, &field_access, offset);
        encode_stmts.push(encode_stmt);
        offset += sol_type.head_size();
    }

    quote! {
        #(#encode_stmts)*
    }
}

fn generate_static_decode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    match fields {
        Fields::Named(named) => {
            let mut offset = 0usize;
            let field_decodes: Vec<_> = named
                .named
                .iter()
                .zip(field_info.iter())
                .map(|(field, (field_name, sol_type))| {
                    let name = field_name.as_ref().unwrap();
                    let ty = &field.ty;
                    let field_offset = offset;
                    offset += sol_type.head_size();
                    quote! {
                        #name: <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + #field_offset)
                    }
                })
                .collect();

            quote! {
                Self { #(#field_decodes),* }
            }
        }
        Fields::Unnamed(unnamed) => {
            let mut offset = 0usize;
            let field_decodes: Vec<_> = unnamed
                .unnamed
                .iter()
                .zip(field_info.iter())
                .map(|(field, (_, sol_type))| {
                    let ty = &field.ty;
                    let field_offset = offset;
                    offset += sol_type.head_size();
                    quote! {
                        <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + #field_offset)
                    }
                })
                .collect();

            quote! {
                Self(#(#field_decodes),*)
            }
        }
        Fields::Unit => quote! { Self },
    }
}

fn generate_dynamic_decode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
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
                    let head_offset = i * 32;
                    let decode = generate_dynamic_field_decode(ty, sol_type, head_offset);
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
                    let head_offset = i * 32;
                    generate_dynamic_field_decode(ty, sol_type, head_offset)
                })
                .collect();

            quote! {
                Self(#(#field_decodes),*)
            }
        }
        Fields::Unit => quote! { Self },
    }
}

fn generate_dynamic_field_decode(ty: &Type, sol_type: &SolType, head_offset: usize) -> TokenStream {
    if sol_type.is_dynamic() {
        let rel_offset_start = head_offset + 24;
        let rel_offset_end = head_offset + 32;
        quote! {{
            let __field_offset =
                u64::from_be_bytes(input[offset + #rel_offset_start..offset + #rel_offset_end].try_into().unwrap())
                    as usize;
            <#ty as ::pvm_contract_types::SolDecode>::decode_tail(input, offset + __field_offset)
        }}
    } else {
        quote! {
            <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + #head_offset)
        }
    }
}

fn generate_field_encode(
    sol_type: &SolType,
    value_expr: &TokenStream,
    offset: usize,
) -> TokenStream {
    match sol_type {
        SolType::Address => {
            quote! {
                buf[#offset..#offset + 12].fill(0);
                buf[#offset + 12..#offset + 32].copy_from_slice(&#value_expr);
            }
        }
        SolType::Bool => {
            quote! {
                buf[#offset..#offset + 31].fill(0);
                buf[#offset + 31] = if #value_expr { 1 } else { 0 };
            }
        }
        SolType::Uint(8) => {
            quote! {
                buf[#offset..#offset + 31].fill(0);
                buf[#offset + 31] = #value_expr;
            }
        }
        SolType::Uint(16) => {
            quote! {
                buf[#offset..#offset + 30].fill(0);
                buf[#offset + 30..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(32) => {
            quote! {
                buf[#offset..#offset + 28].fill(0);
                buf[#offset + 28..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(64) => {
            quote! {
                buf[#offset..#offset + 24].fill(0);
                buf[#offset + 24..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(128) => {
            quote! {
                buf[#offset..#offset + 16].fill(0);
                buf[#offset + 16..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(_) => {
            quote! {
                buf[#offset..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes::<32>());
            }
        }
        SolType::Int(8) => {
            quote! {
                buf[#offset..#offset + 31].fill(if #value_expr < 0 { 0xff } else { 0 });
                buf[#offset + 31] = #value_expr as u8;
            }
        }
        SolType::Int(16) => {
            quote! {
                buf[#offset..#offset + 30].fill(if #value_expr < 0 { 0xff } else { 0 });
                buf[#offset + 30..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(32) => {
            quote! {
                buf[#offset..#offset + 28].fill(if #value_expr < 0 { 0xff } else { 0 });
                buf[#offset + 28..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(64) => {
            quote! {
                buf[#offset..#offset + 24].fill(if #value_expr < 0 { 0xff } else { 0 });
                buf[#offset + 24..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(128) => {
            quote! {
                buf[#offset..#offset + 16].fill(if #value_expr < 0 { 0xff } else { 0 });
                buf[#offset + 16..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(_) => {
            quote! {
                buf[#offset..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes::<32>());
            }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            quote! {
                buf[#offset..#offset + #size_lit].copy_from_slice(&#value_expr);
                buf[#offset + #size_lit..#offset + 32].fill(0);
            }
        }
        SolType::FixedArray(inner, size) => {
            let elem_size = inner.head_size();
            let encode_stmts: Vec<_> = (0..*size)
                .map(|i| {
                    let elem_offset = offset + i * elem_size;
                    let idx = syn::Index::from(i);
                    let elem_expr = quote! { #value_expr[#idx] };
                    generate_field_encode(inner, &elem_expr, elem_offset)
                })
                .collect();
            quote! {
                #(#encode_stmts)*
            }
        }
        SolType::Tuple(types) => {
            let mut current_offset = offset;
            let encode_stmts: Vec<_> = types
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let idx = syn::Index::from(i);
                    let elem_expr = quote! { #value_expr.#idx };
                    let stmt = generate_field_encode(t, &elem_expr, current_offset);
                    current_offset += t.head_size();
                    stmt
                })
                .collect();
            quote! {
                #(#encode_stmts)*
            }
        }
        SolType::String | SolType::DynBytes | SolType::Array(_) | SolType::Custom(_) => {
            quote! {
                ::pvm_contract_types::SolEncode::encode_to(&#value_expr, &mut buf[#offset..]);
            }
        }
    }
}
