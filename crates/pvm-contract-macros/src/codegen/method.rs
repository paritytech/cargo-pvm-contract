use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, ItemFn, LitStr, Token, parse::Parse, parse::ParseStream};

pub struct MethodArgs {
    pub rename: Option<String>,
    pub dyn_len: bool,
}

impl Parse for MethodArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut rename = None;
        let mut dyn_len = false;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            if ident == "rename" {
                input.parse::<Token![=]>()?;
                let name: LitStr = input.parse()?;
                rename = Some(name.value());
            } else if ident == "dyn_len" {
                dyn_len = true;
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

        Ok(MethodArgs { rename, dyn_len })
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
