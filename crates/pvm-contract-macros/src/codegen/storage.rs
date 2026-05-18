use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemStruct, Type};

pub fn expand_storage(input: &ItemStruct) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let vis = &input.vis;

    let syn::Fields::Named(ref fields) = input.fields else {
        return Err(syn::Error::new_spanned(input, "storage macro only supports structs with named fields"));
    };

    let methods: Vec<_> = fields.named.iter().map(|field| {
        let name = &field.ident;
        let ty = &field.ty;
        let fvis = &field.vis;
        let ns = proc_macro2::Literal::byte_string(format!("{}::{}", struct_name, name.as_ref().unwrap()).as_bytes());

        // Storage handle fields are returned directly; all other types get wrapped in Lazy<T>.
        let is_handle = matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Mapping" || s.ident == "OrderedIndex"));
        let (ret_ty, constructor) = if is_handle {
            (quote! { #ty }, quote! { <#ty>::new(#ns) })
        } else {
            (quote! { pvm_contract::storage::Lazy<#ty> }, quote! { pvm_contract::storage::Lazy::new(#ns) })
        };

        quote! { #fvis fn #name() -> #ret_ty { #constructor } }
    }).collect();

    Ok(quote! {
        #vis struct #struct_name;
        impl #struct_name { #(#methods)* }
    })
}
