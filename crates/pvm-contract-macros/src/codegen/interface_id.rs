use proc_macro2::{Literal, TokenStream};
use quote::{format_ident, quote};
use syn::{FnArg, GenericParam, ItemTrait, TraitItem};

use crate::utils::{extract_selector_rename, to_camel_case};

/// Expand `#[interface_id]` on a trait.
///
/// Adds a defaulted associated constant
///
/// ```ignore
/// const INTERFACE_ID: [u8; 4];
/// ```
///
/// equal to the ERC-165 interface ID of the trait: the XOR of the 4-byte
/// Solidity selectors of every method. Selectors follow the usual convention
/// (`keccak256` of the canonical camelCase signature); a method's Solidity name
/// defaults to the camelCase of its Rust name and can be overridden with
/// `#[selector(name = "...")]`.
///
/// Selector text is resolved at const-eval through `<Ty as SolEncode>::SOL_NAME`
/// so custom parameter types work without the macro having to know their ABI
/// name. A separate eager `const _` guard recomputes the selectors and asserts
/// they are pairwise distinct: two methods sharing a selector would silently
/// cancel in the XOR and produce a wrong interface ID, so that is a hard error.
pub fn expand_interface_id(mut input: ItemTrait) -> syn::Result<TokenStream> {
    let trait_ident = input.ident.clone();

    // A generic interface has no single fixed ID: the associated const default
    // would silently take a different value per type parameter. Reject type and
    // const generics (lifetimes are fine — they don't reach a selector).
    if input
        .generics
        .params
        .iter()
        .any(|p| !matches!(p, GenericParam::Lifetime(_)))
    {
        return Err(syn::Error::new(
            trait_ident.span(),
            "#[interface_id]: generic interfaces are not supported; \
             an interface ID must be a single fixed constant",
        ));
    }

    // A trait can't declare INTERFACE_ID itself — the macro owns it.
    for item in &input.items {
        if let TraitItem::Const(c) = item
            && c.ident == "INTERFACE_ID"
        {
            return Err(syn::Error::new(
                c.ident.span(),
                "#[interface_id]: the trait already declares `INTERFACE_ID`; \
                 remove it and let the macro generate it",
            ));
        }
    }

    // Collect each method's Solidity name + parameter types, and strip the inert
    // `#[selector]` helper attrs so the re-emitted trait is clean.
    struct Method {
        sol_name: String,
        param_types: Vec<syn::Type>,
    }
    let mut methods: Vec<Method> = Vec::new();

    for item in &mut input.items {
        let TraitItem::Fn(f) = item else { continue };

        // A selector needs a concrete signature; generic methods have none.
        let generics = &f.sig.generics;
        if generics
            .params
            .iter()
            .any(|p| !matches!(p, GenericParam::Lifetime(_)))
            || generics.where_clause.is_some()
        {
            return Err(syn::Error::new(
                f.sig.ident.span(),
                "#[interface_id]: generic trait methods are not supported \
                 (their selector is undefined)",
            ));
        }

        // Proc-macros run before `#[cfg]` stripping, so a conditionally-compiled
        // method still contributes its selector to the XOR even when it is
        // configured out. That yields an INTERFACE_ID that doesn't match the
        // active method set — reject it, like the generic-interface case above.
        for attr in &f.attrs {
            if attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr") {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[interface_id]: `#[cfg]` / `#[cfg_attr]` on interface methods is \
                     not supported; a conditionally-compiled method would still \
                     contribute its selector to the interface ID",
                ));
            }
        }

        // `impl Trait` in argument position isn't a `sig.generics` param, so the
        // check above misses it; reject it here for a clear diagnostic.
        for arg in &f.sig.inputs {
            if let FnArg::Typed(pat) = arg
                && matches!(&*pat.ty, syn::Type::ImplTrait(_))
            {
                return Err(syn::Error::new_spanned(
                    &pat.ty,
                    "#[interface_id]: `impl Trait` parameters are not supported \
                     (their selector type is undefined)",
                ));
            }
        }

        let sol_name = match extract_selector_rename(&f.attrs)? {
            Some(name) => name,
            None => to_camel_case(&f.sig.ident.to_string()),
        };

        let param_types = f
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Receiver(_) => None,
                FnArg::Typed(pat) => Some((*pat.ty).clone()),
            })
            .collect();

        f.attrs.retain(|a| {
            a.path().segments.last().map(|s| s.ident.to_string()) != Some("selector".to_string())
        });

        methods.push(Method {
            sol_name,
            param_types,
        });
    }

    if methods.is_empty() {
        return Err(syn::Error::new(
            trait_ident.span(),
            "#[interface_id]: the trait has no methods; an interface ID needs at least one",
        ));
    }

    // Per-method selector consts, deferred to const-eval so `SOL_NAME` resolves
    // for custom parameter types. Reused by the value and the guard below.
    let sel_idents: Vec<_> = (0..methods.len())
        .map(|i| format_ident!("__IID_SEL_{}", i))
        .collect();
    let sel_consts: Vec<TokenStream> = methods
        .iter()
        .zip(&sel_idents)
        .map(|(m, id)| {
            let sig_expr = build_signature_expr(&m.sol_name, &m.param_types);
            quote! {
                const #id: [u8; 4] = ::pvm_contract_sdk::const_selector(#sig_expr);
            }
        })
        .collect();

    // INTERFACE_ID = XOR of every selector, byte by byte.
    let xor_bytes: Vec<TokenStream> = (0..4)
        .map(|b| {
            let idx = Literal::usize_unsuffixed(b);
            let terms = sel_idents.iter().map(|id| quote! { #id[#idx] });
            quote! { #(#terms)^* }
        })
        .collect();

    let interface_id_const: TraitItem = syn::parse_quote! {
        const INTERFACE_ID: [u8; 4] = {
            #(#sel_consts)*
            [ #(#xor_bytes),* ]
        };
    };
    input.items.push(interface_id_const);

    // Eager duplicate-selector guard. `!=` on `[u8; 4]` isn't const, so compare
    // the selectors as big-endian u32s (both `from_be_bytes` and `!=` are const).
    let mut pairs: Vec<TokenStream> = Vec::new();
    for i in 0..sel_idents.len() {
        for j in (i + 1)..sel_idents.len() {
            let a = &sel_idents[i];
            let b = &sel_idents[j];
            pairs.push(quote! {
                ::core::primitive::u32::from_be_bytes(#a)
                    != ::core::primitive::u32::from_be_bytes(#b)
            });
        }
    }
    let uniqueness = if pairs.is_empty() {
        quote! { true }
    } else {
        quote! { #(#pairs)&&* }
    };
    let dup_msg = format!(
        "#[interface_id]: two methods of trait `{trait_ident}` produce the same 4-byte \
         selector; they would cancel in the interface-ID XOR. Rename one with \
         #[selector(name = \"...\")]"
    );
    let guard = quote! {
        const _: () = {
            #(#sel_consts)*
            ::core::assert!(#uniqueness, #dup_msg);
        };
    };

    Ok(quote! {
        #input
        #guard
    })
}

/// Build the const-eval expression for a method's canonical Solidity signature,
/// e.g. `concatcp!("transfer(", <Address>::SOL_NAME, ",", <U256>::SOL_NAME, ")")`.
fn build_signature_expr(sol_name: &str, params: &[syn::Type]) -> TokenStream {
    let prefix = format!("{sol_name}(");
    let mut parts: Vec<TokenStream> = vec![quote! { #prefix }];
    for (i, ty) in params.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        parts.push(quote! { <#ty as ::pvm_contract_sdk::SolEncode>::SOL_NAME });
    }
    parts.push(quote! { ")" });
    quote! { ::pvm_contract_sdk::const_format::concatcp!(#(#parts),*) }
}
