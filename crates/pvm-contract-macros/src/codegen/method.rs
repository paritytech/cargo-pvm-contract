use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse::Parse, parse::ParseStream, Ident, ItemFn, ItemStruct, LitStr, Token, Type};

pub struct MethodArgs {
    pub rename: Option<String>,
}

impl Parse for MethodArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut rename = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            if ident == "rename" {
                input.parse::<Token![=]>()?;
                let name: LitStr = input.parse()?;
                rename = Some(name.value());
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("Unknown attribute: {}", ident),
                ));
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(MethodArgs { rename })
    }
}

pub fn expand_method(args: MethodArgs, input: ItemFn) -> syn::Result<TokenStream> {
    let _ = args.rename;

    let fn_name = &input.sig.ident;
    let fn_vis = &input.vis;
    let fn_block = &input.block;
    let fn_inputs = &input.sig.inputs;
    let fn_output = &input.sig.output;

    Ok(quote! {
        #fn_vis fn #fn_name(#fn_inputs) #fn_output #fn_block
    })
}

pub fn expand_constructor(input: ItemFn) -> syn::Result<TokenStream> {
    let fn_name = &input.sig.ident;
    let fn_vis = &input.vis;
    let fn_block = &input.block;
    let fn_inputs = &input.sig.inputs;
    let fn_output = &input.sig.output;

    Ok(quote! {
        #fn_vis fn #fn_name(#fn_inputs) #fn_output #fn_block
    })
}

pub fn expand_fallback(input: ItemFn) -> syn::Result<TokenStream> {
    let fn_name = &input.sig.ident;
    let fn_vis = &input.vis;
    let fn_block = &input.block;
    let fn_inputs = &input.sig.inputs;
    let fn_output = &input.sig.output;

    Ok(quote! {
        #fn_vis fn #fn_name(#fn_inputs) #fn_output #fn_block
    })
}

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

        // Mapping fields are returned directly; all other types get wrapped in Lazy<T>
        let (ret_ty, constructor) = if matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Mapping")) {
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
