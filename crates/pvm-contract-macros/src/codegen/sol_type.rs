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
            ))
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "SolType can only be derived for structs",
            ))
        }
    };

    let field_info = extract_field_info(fields)?;

    if field_info.is_empty() {
        return Err(syn::Error::new_spanned(
            input,
            "SolType requires at least one field",
        ));
    }

    for (_, sol_type) in &field_info {
        if sol_type.is_dynamic() {
            return Err(syn::Error::new_spanned(
                &input,
                "SolType derive does not support dynamic types (bytes, string, dynamic arrays). Use fixed-size types only.",
            ));
        }
    }

    let total_size: usize = field_info.iter().map(|(_, t)| t.head_size()).sum();
    let sol_name = build_sol_signature(&field_info);

    let encode_body = generate_encode_body(fields, &field_info);

    let expanded = quote! {
        impl ::pvm_contract_types::SolEncode for #name {
            const SOL_NAME: &'static str = #sol_name;
            const ENCODED_SIZE: usize = #total_size;

            #[inline]
            fn sol_encode_to(&self, buf: &mut [u8]) {
                #encode_body
            }
        }
    };

    Ok(expanded)
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
            format!(
                "Unsupported type for SolType derive. Supported types: \
                 U256, u128, u64, u32, u16, u8, i128, i64, i32, i16, i8, \
                 bool, [u8; 20] (address), [u8; N] (bytesN). \
                 For custom structs, derive SolType on them first."
            ),
        )
    })
}

fn build_sol_signature(field_info: &[(Option<syn::Ident>, SolType)]) -> String {
    let types: Vec<String> = field_info.iter().map(|(_, t)| t.canonical_name()).collect();
    format!("({})", types.join(","))
}

fn generate_encode_body(
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
        _ => panic!("Dynamic types not supported in SolType derive"),
    }
}
