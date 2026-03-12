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
    let encode_body = generate_static_encode_body(fields, field_info);
    let decode_body = generate_static_decode_body(fields, field_info);

    let size_parts: Vec<TokenStream> = field_info
        .iter()
        .map(|(_, sol_type)| field_size_expr(sol_type))
        .collect();
    let total_size_expr = quote! { 0 #(+ #size_parts)* };

    Ok(quote! {
        impl ::pvm_contract_types::SolEncode for #name {
            const IS_DYNAMIC: bool = false;

            #[inline]
            fn encode_len(&self) -> usize {
                <Self as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE
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
            const ENCODED_SIZE: usize = #total_size_expr;
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

    let head_parts: Vec<TokenStream> = field_info
        .iter()
        .map(|(_, sol_type)| dynamic_field_head_expr(sol_type))
        .collect();
    let head_size_expr = quote! { 0 #(+ #head_parts)* };

    let encode_len_body = generate_dynamic_encode_len(fields, field_info, &head_size_expr);
    let encode_body = generate_dynamic_encode_body(fields, field_info, &head_size_expr);
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

/// Size expression for a field. Custom types use a const expression resolved at compile time.
fn field_size_expr(sol_type: &SolType) -> TokenStream {
    match sol_type {
        SolType::Custom(name) => {
            let type_path: syn::Path = syn::parse_str(name).unwrap();
            quote! { <#type_path as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE }
        }
        _ => {
            let size = sol_type.head_size();
            quote! { #size }
        }
    }
}

/// Head contribution of a single field inside a dynamic struct.
/// Dynamic fields contribute 32 bytes (offset pointer).
/// Static fields contribute their full encoded size.
fn dynamic_field_head_expr(sol_type: &SolType) -> TokenStream {
    if sol_type.is_dynamic() {
        quote! { 32usize }
    } else {
        field_size_expr(sol_type)
    }
}

fn build_sol_signature(field_info: &[(Option<syn::Ident>, SolType)]) -> String {
    let field_types = field_info
        .iter()
        .map(|(_, sol_type)| sol_type.canonical_name())
        .collect::<Vec<_>>();
    format!("({})", field_types.join(","))
}

fn field_access_expr(
    fields: &Fields,
    i: usize,
    field_name: &Option<syn::Ident>,
) -> Option<TokenStream> {
    match fields {
        Fields::Named(_) => {
            let name = field_name.as_ref().unwrap();
            Some(quote! { self.#name })
        }
        Fields::Unnamed(_) => {
            let idx = syn::Index::from(i);
            Some(quote! { self.#idx })
        }
        Fields::Unit => None,
    }
}

fn generate_static_encode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    let mut encode_stmts = Vec::new();
    for (i, (field_name, sol_type)) in field_info.iter().enumerate() {
        let Some(field_access) = field_access_expr(fields, i, field_name) else {
            continue;
        };
        let size_expr = field_size_expr(sol_type);
        encode_stmts.push(quote! {
            ::pvm_contract_types::SolEncode::encode_to(&#field_access, &mut buf[__offset..]);
            __offset += #size_expr;
        });
    }

    quote! {
        let mut __offset: usize = 0;
        #(#encode_stmts)*
    }
}

fn generate_static_decode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
) -> TokenStream {
    match fields {
        Fields::Named(named) => {
            let field_decodes: Vec<_> = named
                .named
                .iter()
                .zip(field_info.iter())
                .map(|(field, (field_name, sol_type))| {
                    let name = field_name.as_ref().unwrap();
                    let ty = &field.ty;
                    let size_expr = field_size_expr(sol_type);
                    quote! {
                        #name: {
                            let __val = <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __dec_offset);
                            __dec_offset += #size_expr;
                            __val
                        }
                    }
                })
                .collect();

            quote! {
                let mut __dec_offset: usize = 0;
                Self { #(#field_decodes),* }
            }
        }
        Fields::Unnamed(unnamed) => {
            let field_decodes: Vec<_> = unnamed
                .unnamed
                .iter()
                .zip(field_info.iter())
                .map(|(field, (_, sol_type))| {
                    let ty = &field.ty;
                    let size_expr = field_size_expr(sol_type);
                    quote! {
                        {
                            let __val = <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __dec_offset);
                            __dec_offset += #size_expr;
                            __val
                        }
                    }
                })
                .collect();

            quote! {
                let mut __dec_offset: usize = 0;
                Self(#(#field_decodes),*)
            }
        }
        Fields::Unit => quote! { Self },
    }
}

fn generate_dynamic_encode_len(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size_expr: &TokenStream,
) -> TokenStream {
    let tail_lens: Vec<TokenStream> = field_info
        .iter()
        .enumerate()
        .filter_map(|(i, (field_name, sol_type))| {
            if !sol_type.is_dynamic() {
                return None;
            }
            let field_access = field_access_expr(fields, i, field_name)?;
            Some(quote! {
                ::pvm_contract_types::SolEncode::tail_len(&#field_access)
            })
        })
        .collect();

    quote! {
        #head_size_expr #(+ #tail_lens)*
    }
}

fn generate_dynamic_encode_body(
    fields: &Fields,
    field_info: &[(Option<syn::Ident>, SolType)],
    head_size_expr: &TokenStream,
) -> TokenStream {
    let mut stmts = Vec::new();

    for (i, (field_name, sol_type)) in field_info.iter().enumerate() {
        let Some(field_access) = field_access_expr(fields, i, field_name) else {
            continue;
        };
        let head_expr = dynamic_field_head_expr(sol_type);

        if sol_type.is_dynamic() {
            stmts.push(quote! {
                buf[__head_offset..__head_offset + 24].fill(0);
                buf[__head_offset + 24..__head_offset + 32].copy_from_slice(&(__tail_offset as u64).to_be_bytes());
                __head_offset += #head_expr;
                let __tail_len = ::pvm_contract_types::SolEncode::tail_len(&#field_access);
                ::pvm_contract_types::SolEncode::encode_tail_to(&#field_access, &mut buf[__tail_offset..__tail_offset + __tail_len]);
                __tail_offset += __tail_len;
            });
        } else {
            stmts.push(quote! {
                ::pvm_contract_types::SolEncode::encode_to(&#field_access, &mut buf[__head_offset..]);
                __head_offset += #head_expr;
            });
        }
    }

    quote! {
        let mut __head_offset: usize = 0;
        let mut __tail_offset: usize = #head_size_expr;
        #(#stmts)*
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
                .map(|(field, (field_name, sol_type))| {
                    let name = field_name.as_ref().unwrap();
                    let ty = &field.ty;
                    let head_expr = dynamic_field_head_expr(sol_type);
                    if sol_type.is_dynamic() {
                        quote! {
                            #name: {
                                let __field_offset =
                                    u64::from_be_bytes(input[offset + __dec_offset + 24..offset + __dec_offset + 32].try_into().unwrap())
                                        as usize;
                                __dec_offset += #head_expr;
                                <#ty as ::pvm_contract_types::SolDecode>::decode_tail(input, offset + __field_offset)
                            }
                        }
                    } else {
                        quote! {
                            #name: {
                                let __val = <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __dec_offset);
                                __dec_offset += #head_expr;
                                __val
                            }
                        }
                    }
                })
                .collect();

            quote! {
                let mut __dec_offset: usize = 0;
                Self { #(#field_decodes),* }
            }
        }
        Fields::Unnamed(unnamed) => {
            let field_decodes: Vec<_> = unnamed
                .unnamed
                .iter()
                .zip(field_info.iter())
                .map(|(field, (_, sol_type))| {
                    let ty = &field.ty;
                    let head_expr = dynamic_field_head_expr(sol_type);
                    if sol_type.is_dynamic() {
                        quote! {{
                            let __field_offset =
                                u64::from_be_bytes(input[offset + __dec_offset + 24..offset + __dec_offset + 32].try_into().unwrap())
                                    as usize;
                            __dec_offset += #head_expr;
                            <#ty as ::pvm_contract_types::SolDecode>::decode_tail(input, offset + __field_offset)
                        }}
                    } else {
                        quote! {{
                            let __val = <#ty as ::pvm_contract_types::SolDecode>::decode_at(input, offset + __dec_offset);
                            __dec_offset += #head_expr;
                            __val
                        }}
                    }
                })
                .collect();

            quote! {
                let mut __dec_offset: usize = 0;
                Self(#(#field_decodes),*)
            }
        }
        Fields::Unit => quote! { Self },
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
