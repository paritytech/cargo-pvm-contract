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

    let decode_body = generate_decode_body(fields, &field_info);
    let encode_body = generate_encode_body(fields, &field_info, total_size);

    let expanded = quote! {
        impl #name {
            /// The Solidity type signature for ABI encoding
            pub const SOL_NAME: &'static str = #sol_name;

            /// The ABI-encoded size in bytes
            pub const ENCODED_SIZE: usize = #total_size;

            /// Decode from ABI-encoded bytes at the given offset
            #[inline]
            pub fn abi_decode(input: &[u8], offset: usize) -> Self {
                #decode_body
            }

            /// Encode to ABI format (fixed-size output)
            #[inline]
            pub fn abi_encode(&self) -> [u8; #total_size] {
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

fn generate_decode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let mut offset = 0usize;
    let mut field_decodes = Vec::new();

    match fields {
        Fields::Named(_) => {
            for (field_name, sol_type) in field_info {
                let name = field_name.as_ref().unwrap();
                let decode_expr = generate_field_decode(sol_type, offset);
                field_decodes.push(quote! { #name: #decode_expr });
                offset += sol_type.head_size();
            }
            quote! {
                Self {
                    #(#field_decodes),*
                }
            }
        }
        Fields::Unnamed(_) => {
            for (_, sol_type) in field_info {
                let decode_expr = generate_field_decode(sol_type, offset);
                field_decodes.push(decode_expr);
                offset += sol_type.head_size();
            }
            quote! {
                Self(#(#field_decodes),*)
            }
        }
        Fields::Unit => quote! { Self },
    }
}

fn generate_field_decode(sol_type: &SolType, offset: usize) -> TokenStream {
    match sol_type {
        SolType::Address => {
            quote! {{
                let mut addr = [0u8; 20];
                addr.copy_from_slice(&input[offset + #offset + 12..offset + #offset + 32]);
                addr
            }}
        }
        SolType::Bool => {
            quote! {
                input[offset + #offset + 31] != 0
            }
        }
        SolType::Uint(8) => {
            quote! {
                input[offset + #offset + 31]
            }
        }
        SolType::Uint(16) => {
            quote! {
                u16::from_be_bytes([input[offset + #offset + 30], input[offset + #offset + 31]])
            }
        }
        SolType::Uint(32) => {
            quote! {
                u32::from_be_bytes(input[offset + #offset + 28..offset + #offset + 32].try_into().unwrap())
            }
        }
        SolType::Uint(64) => {
            quote! {
                u64::from_be_bytes(input[offset + #offset + 24..offset + #offset + 32].try_into().unwrap())
            }
        }
        SolType::Uint(128) => {
            quote! {
                u128::from_be_bytes(input[offset + #offset + 16..offset + #offset + 32].try_into().unwrap())
            }
        }
        SolType::Uint(_) => {
            quote! {
                ruint::aliases::U256::from_be_slice(&input[offset + #offset..offset + #offset + 32])
            }
        }
        SolType::Int(8) => {
            quote! {
                input[offset + #offset + 31] as i8
            }
        }
        SolType::Int(16) => {
            quote! {
                i16::from_be_bytes([input[offset + #offset + 30], input[offset + #offset + 31]])
            }
        }
        SolType::Int(32) => {
            quote! {
                i32::from_be_bytes(input[offset + #offset + 28..offset + #offset + 32].try_into().unwrap())
            }
        }
        SolType::Int(64) => {
            quote! {
                i64::from_be_bytes(input[offset + #offset + 24..offset + #offset + 32].try_into().unwrap())
            }
        }
        SolType::Int(128) => {
            quote! {
                i128::from_be_bytes(input[offset + #offset + 16..offset + #offset + 32].try_into().unwrap())
            }
        }
        SolType::Int(_) => {
            quote! {
                ruint::aliases::I256::from_be_slice(&input[offset + #offset..offset + #offset + 32])
            }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            quote! {{
                let mut bytes = [0u8; #size_lit];
                bytes.copy_from_slice(&input[offset + #offset..offset + #offset + #size_lit]);
                bytes
            }}
        }
        SolType::FixedArray(inner, size) => {
            let elem_size = inner.head_size();
            let elem_decodes: Vec<_> = (0..*size)
                .map(|i| {
                    let elem_offset = offset + i * elem_size;
                    generate_field_decode(inner, elem_offset)
                })
                .collect();
            quote! {
                [#(#elem_decodes),*]
            }
        }
        SolType::Tuple(types) => {
            let mut current_offset = offset;
            let elem_decodes: Vec<_> = types
                .iter()
                .map(|t| {
                    let decode = generate_field_decode(t, current_offset);
                    current_offset += t.head_size();
                    decode
                })
                .collect();
            quote! {
                (#(#elem_decodes),*)
            }
        }
        _ => panic!("Dynamic types not supported in SolType derive"),
    }
}

fn generate_encode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    total_size: usize,
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
        let mut out = [0u8; #total_size];
        #(#encode_stmts)*
        out
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
                out[#offset + 12..#offset + 32].copy_from_slice(&#value_expr);
            }
        }
        SolType::Bool => {
            quote! {
                out[#offset + 31] = if #value_expr { 1 } else { 0 };
            }
        }
        SolType::Uint(8) => {
            quote! {
                out[#offset + 31] = #value_expr;
            }
        }
        SolType::Uint(16) => {
            quote! {
                out[#offset + 30..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(32) => {
            quote! {
                out[#offset + 28..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(64) => {
            quote! {
                out[#offset + 24..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(128) => {
            quote! {
                out[#offset + 16..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Uint(_) => {
            quote! {
                out[#offset..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes::<32>());
            }
        }
        SolType::Int(8) => {
            quote! {
                if #value_expr < 0 {
                    out[#offset..#offset + 31].fill(0xff);
                }
                out[#offset + 31] = #value_expr as u8;
            }
        }
        SolType::Int(16) => {
            quote! {
                if #value_expr < 0 {
                    out[#offset..#offset + 30].fill(0xff);
                }
                out[#offset + 30..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(32) => {
            quote! {
                if #value_expr < 0 {
                    out[#offset..#offset + 28].fill(0xff);
                }
                out[#offset + 28..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(64) => {
            quote! {
                if #value_expr < 0 {
                    out[#offset..#offset + 24].fill(0xff);
                }
                out[#offset + 24..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(128) => {
            quote! {
                if #value_expr < 0 {
                    out[#offset..#offset + 16].fill(0xff);
                }
                out[#offset + 16..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes());
            }
        }
        SolType::Int(_) => {
            quote! {
                out[#offset..#offset + 32].copy_from_slice(&#value_expr.to_be_bytes::<32>());
            }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            quote! {
                out[#offset..#offset + #size_lit].copy_from_slice(&#value_expr);
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
