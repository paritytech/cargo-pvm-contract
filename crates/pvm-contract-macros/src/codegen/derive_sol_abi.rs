use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

pub fn expand_derive_sol_abi(input: DeriveInput) -> Result<TokenStream, syn::Error> {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    match &input.data {
        Data::Struct(data) => match &data.fields {
            // Newtype: struct Foo(pub InnerType)
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let inner_ty = &fields.unnamed[0].ty;
                Ok(expand_newtype(name, &impl_generics, &ty_generics, where_clause, inner_ty))
            }
            // Named struct: struct Foo { x: u64, y: u64 }
            Fields::Named(fields) => {
                expand_named_struct(name, &impl_generics, &ty_generics, where_clause, fields)
            }
            Fields::Unnamed(fields) => {
                Err(syn::Error::new_spanned(
                    fields,
                    "SolAbi can only be derived for newtypes (single-field tuple structs) or named structs",
                ))
            }
            Fields::Unit => {
                Err(syn::Error::new_spanned(
                    name,
                    "SolAbi cannot be derived for unit structs",
                ))
            }
        },
        Data::Enum(_) => Err(syn::Error::new_spanned(
            name,
            "SolAbi cannot be derived for enums",
        )),
        Data::Union(_) => Err(syn::Error::new_spanned(
            name,
            "SolAbi cannot be derived for unions",
        )),
    }
}

use super::unwrap_option_inner;

fn expand_newtype(
    name: &syn::Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
    inner_ty: &syn::Type,
) -> TokenStream {
    // If the newtype wraps Option<T>, resolve the const strings through the inner type
    let (sol_name_expr, abi_type_expr, abi_components_expr) =
        if let Some(inner) = unwrap_option_inner(inner_ty) {
            (
                quote! {
                    pvm_contract::const_format::concatcp!(
                        "(bool,", <#inner as pvm_contract::SolAbi>::SOL_NAME, ")"
                    )
                },
                quote! { "tuple" },
                quote! {
                    pvm_contract::const_format::concatcp!(
                        ",\"components\":[{\"name\":\"isSome\",\"type\":\"bool\"},{\"name\":\"value\",\"type\":\"",
                        <#inner as pvm_contract::SolAbi>::ABI_TYPE,
                        "\"",
                        <#inner as pvm_contract::SolAbi>::ABI_COMPONENTS,
                        "}]"
                    )
                },
            )
        } else {
            (
                quote! { <#inner_ty as pvm_contract::SolAbi>::SOL_NAME },
                quote! { <#inner_ty as pvm_contract::SolAbi>::ABI_TYPE },
                quote! { <#inner_ty as pvm_contract::SolAbi>::ABI_COMPONENTS },
            )
        };

    quote! {
        impl #impl_generics pvm_contract::SolAbi for #name #ty_generics #where_clause {
            const SOL_NAME: &'static str = #sol_name_expr;
            const ABI_TYPE: &'static str = #abi_type_expr;
            const ABI_COMPONENTS: &'static str = #abi_components_expr;
            const HEAD_SIZE: usize = <#inner_ty as pvm_contract::SolAbi>::HEAD_SIZE;
            const IS_DYNAMIC: bool = <#inner_ty as pvm_contract::SolAbi>::IS_DYNAMIC;

            fn abi_encode(&self, buf: &mut alloc::vec::Vec<u8>) {
                <#inner_ty as pvm_contract::SolAbi>::abi_encode(&self.0, buf)
            }

            fn abi_decode(data: &[u8], offset: usize) -> Self {
                Self(<#inner_ty as pvm_contract::SolAbi>::abi_decode(data, offset))
            }
        }
    }
}

fn expand_named_struct(
    name: &syn::Ident,
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
    fields: &syn::FieldsNamed,
) -> Result<TokenStream, syn::Error> {
    let mut field_idents: Vec<syn::Ident> = Vec::new();
    let mut field_types: Vec<syn::Type> = Vec::new();

    for field in &fields.named {
        let ident = field.ident.as_ref().unwrap();
        field_idents.push(ident.clone());
        field_types.push(field.ty.clone());
    }

    // Build SOL_NAME using concatcp! so it works with trait-delegated names (nested structs)
    // For Option<T> fields, resolve through the inner type to avoid placeholder const strings
    let sol_name_parts: Vec<TokenStream> = field_types.iter().enumerate().map(|(i, ty)| {
        let comma = if i > 0 { quote! { "," } } else { quote! {} };
        let type_name = if let Some(inner) = unwrap_option_inner(ty) {
            quote! { "(bool,", <#inner as pvm_contract::SolAbi>::SOL_NAME, ")" }
        } else {
            quote! { <#ty as pvm_contract::SolAbi>::SOL_NAME }
        };
        if i > 0 {
            quote! { #comma, #type_name }
        } else {
            quote! { #type_name }
        }
    }).collect();

    // Build ABI_COMPONENTS: ,"components":[{field entries}]
    // For Option<T> fields, inline the tuple component structure
    let component_parts: Vec<TokenStream> = field_idents.iter().zip(field_types.iter()).enumerate().map(|(i, (ident, ty))| {
        let field_name = ident.to_string();
        let comma = if i > 0 {
            quote! { ",", }
        } else {
            quote! {}
        };
        if let Some(inner) = unwrap_option_inner(ty) {
            quote! {
                #comma
                "{\"name\":\"", #field_name, "\",\"type\":\"tuple\"",
                ",\"components\":[{\"name\":\"isSome\",\"type\":\"bool\"},{\"name\":\"value\",\"type\":\"",
                <#inner as pvm_contract::SolAbi>::ABI_TYPE,
                "\"",
                <#inner as pvm_contract::SolAbi>::ABI_COMPONENTS,
                "}]}"
            }
        } else {
            quote! {
                #comma
                "{\"name\":\"", #field_name, "\",\"type\":\"",
                <#ty as pvm_contract::SolAbi>::ABI_TYPE,
                "\"",
                <#ty as pvm_contract::SolAbi>::ABI_COMPONENTS,
                "}"
            }
        }
    }).collect();

    // Compute HEAD_SIZE as sum of field head sizes
    let head_size_expr: Vec<_> = field_types
        .iter()
        .map(|ty| {
            quote! { <#ty as pvm_contract::SolAbi>::HEAD_SIZE }
        })
        .collect();

    // IS_DYNAMIC: true if any field is dynamic
    let is_dynamic_expr: Vec<_> = field_types
        .iter()
        .map(|ty| {
            quote! { <#ty as pvm_contract::SolAbi>::IS_DYNAMIC }
        })
        .collect();

    // For encode/decode we need repeated idents/types
    let field_idents2 = &field_idents;
    let field_types2 = &field_types;
    let field_idents3 = &field_idents;
    let field_types3 = &field_types;
    let field_idents4 = &field_idents;
    let field_types4 = &field_types;
    let field_types5 = &field_types;

    let base_impl = quote! {
        impl #impl_generics pvm_contract::SolAbi for #name #ty_generics #where_clause {
            const SOL_NAME: &'static str = pvm_contract::const_format::concatcp!(
                "(", #(#sol_name_parts,)* ")"
            );
            const ABI_TYPE: &'static str = "tuple";
            const ABI_COMPONENTS: &'static str = pvm_contract::const_format::concatcp!(
                ",\"components\":[",
                #(#component_parts,)*
                "]"
            );
            const HEAD_SIZE: usize = #( #head_size_expr )+*;
            const IS_DYNAMIC: bool = #( #is_dynamic_expr )||*;

            fn abi_encode(&self, buf: &mut alloc::vec::Vec<u8>) {
                if !<Self as pvm_contract::SolAbi>::IS_DYNAMIC {
                    #( <#field_types2 as pvm_contract::SolAbi>::abi_encode(&self.#field_idents2, buf); )*
                } else {
                    let head_start = buf.len();
                    let total_head: usize = 0 #(+ <#field_types3 as pvm_contract::SolAbi>::HEAD_SIZE)*;
                    buf.resize(head_start + total_head, 0);
                    let mut hp = head_start;
                    let mut tail = alloc::vec::Vec::new();
                    #(
                        if <#field_types4 as pvm_contract::SolAbi>::IS_DYNAMIC {
                            let off = (total_head + tail.len()) as u64;
                            buf[hp + 24..hp + 32].copy_from_slice(&off.to_be_bytes());
                            hp += 32;
                            <#field_types4 as pvm_contract::SolAbi>::abi_encode(&self.#field_idents3, &mut tail);
                        } else {
                            let mut tmp = alloc::vec::Vec::new();
                            <#field_types4 as pvm_contract::SolAbi>::abi_encode(&self.#field_idents3, &mut tmp);
                            buf[hp..hp + tmp.len()].copy_from_slice(&tmp);
                            hp += <#field_types4 as pvm_contract::SolAbi>::HEAD_SIZE;
                        }
                    )*
                    buf.extend_from_slice(&tail);
                }
            }

            fn abi_decode(data: &[u8], offset: usize) -> Self {
                let sd = if <Self as pvm_contract::SolAbi>::IS_DYNAMIC {
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&data[offset..offset + 32]).as_limbs()[0]
                            as usize;
                    &data[dyn_offset..]
                } else {
                    &data[offset..]
                };
                let mut hp = 0usize;
                Self {
                    #( #field_idents4: {
                        let val = <#field_types5 as pvm_contract::SolAbi>::abi_decode(sd, hp);
                        hp += <#field_types5 as pvm_contract::SolAbi>::HEAD_SIZE;
                        val
                    } ),*
                }
            }
        }
    };
    Ok(base_impl)
}
