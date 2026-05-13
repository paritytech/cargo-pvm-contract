//! `#[interface_id]` attribute macro: adds an `interface_id() -> [u8; 4]`
//! provided method to a trait.
//!
//! The interface ID matches Solidity's ERC-165 convention: the XOR of the
//! function selectors (first 4 bytes of `keccak256(canonical_signature)`) of
//! every method declared on the trait.
//!
//! Each method's signature is built at compile time from the Rust parameter
//! types via their [`SolEncode::SOL_NAME`] constants and the method's name in
//! camelCase. The resulting `interface_id()` body is `const`-friendly: every
//! per-method selector is computed in a `const` block; only the final XOR
//! reduction happens at runtime, which is negligible because `interface_id()`
//! is itself rarely called (only from `supportsInterface`).
//!
//! # Example
//!
//! ```ignore
//! #[pvm_contract_sdk::interface_id]
//! pub trait IErc20 {
//!     fn total_supply(&self) -> U256;
//!     fn balance_of(&self, account: Address) -> U256;
//!     fn transfer(&mut self, to: Address, value: U256) -> Result<bool, Self::Error>;
//! }
//! ```
//!
//! generates (roughly):
//!
//! ```ignore
//! pub trait IErc20 {
//!     fn total_supply(&self) -> U256;
//!     fn balance_of(&self, account: Address) -> U256;
//!     fn transfer(&mut self, to: Address, value: U256) -> Result<bool, Self::Error>;
//!
//!     fn interface_id() -> [u8; 4] where Self: Sized {
//!         let mut id = [0u8; 4];
//!         const SEL_0: [u8; 4] = ::pvm_contract_sdk::const_selector(
//!             ::pvm_contract_sdk::const_format::concatcp!("totalSupply", "(", ")")
//!         );
//!         id[0] ^= SEL_0[0]; id[1] ^= SEL_0[1]; id[2] ^= SEL_0[2]; id[3] ^= SEL_0[3];
//!         /* ...one block per method... */
//!         id
//!     }
//! }
//! ```
//!
//! # Method names and renames
//!
//! Rust `snake_case` names are converted to Solidity `camelCase` (e.g.
//! `balance_of` → `balanceOf`). Explicit renames are supported via
//! `#[selector(name = "exactSolidityName")]` on individual trait methods.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemTrait, LitStr, Token, TraitItem};

use crate::utils::to_camel_case;

/// Top-level entry point. Validates the trait has at least one method and
/// generates the augmented trait with `interface_id()` appended.
pub fn expand_interface_id(input: ItemTrait) -> syn::Result<TokenStream> {
    let methods: Vec<&syn::TraitItemFn> = input
        .items
        .iter()
        .filter_map(|item| match item {
            TraitItem::Fn(f) => Some(f),
            _ => None,
        })
        .collect();

    if methods.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[interface_id] requires at least one method on the trait",
        ));
    }

    // Build a (selector_const_ident, selector_const_decl) pair per method.
    let mut method_blocks: Vec<TokenStream> = Vec::with_capacity(methods.len());
    let mut xor_lines: Vec<TokenStream> = Vec::with_capacity(methods.len());

    for (i, func) in methods.iter().enumerate() {
        // Determine the Solidity name: explicit `#[selector(name = "...")]`
        // wins; otherwise convert snake_case → camelCase.
        let solidity_name = extract_selector_rename(&func.attrs)?
            .unwrap_or_else(|| to_camel_case(&func.sig.ident.to_string()));

        // Collect typed parameters (skip `&self`/`&mut self`/`self`).
        let param_types: Vec<&syn::Type> = func
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(pat) => Some(&*pat.ty),
                FnArg::Receiver(_) => None,
            })
            .collect();

        // Build the canonical signature as a const concatenation:
        // "<name>(" + <Ty0 SOL_NAME> + "," + <Ty1 SOL_NAME> + ... + ")"
        let sig_concat = build_signature_concat(&solidity_name, &param_types);

        let const_ident = format_ident!("__pvm_interface_id_sel_{}", i);
        method_blocks.push(quote! {
            #[allow(non_upper_case_globals)]
            const #const_ident: [u8; 4] = ::pvm_contract_sdk::const_selector(#sig_concat);
        });
        xor_lines.push(quote! {
            id[0] ^= #const_ident[0];
            id[1] ^= #const_ident[1];
            id[2] ^= #const_ident[2];
            id[3] ^= #const_ident[3];
        });
    }

    // Strip our `#[selector(name = "...")]` attributes from the trait items
    // before re-emitting; they are processed here and not by rustc.
    let mut output_trait = input.clone();
    for item in output_trait.items.iter_mut() {
        if let TraitItem::Fn(f) = item {
            f.attrs.retain(|a| !a.path().is_ident("selector"));
        }
    }

    // Append the interface_id() provided method.
    let trait_attrs = &output_trait.attrs;
    let trait_vis = &output_trait.vis;
    let trait_unsafety = &output_trait.unsafety;
    let trait_ident = &output_trait.ident;
    let (impl_generics, _ty_generics, where_clause) = output_trait.generics.split_for_impl();
    let _ = impl_generics; // generics are already attached to the trait token below
    let supertraits = if output_trait.supertraits.is_empty() {
        quote! {}
    } else {
        let st = &output_trait.supertraits;
        quote! { : #st }
    };
    let generics = &output_trait.generics;
    let trait_items = &output_trait.items;

    Ok(quote! {
        #(#trait_attrs)*
        #trait_vis #trait_unsafety trait #trait_ident #generics #supertraits #where_clause {
            #(#trait_items)*

            #[doc = concat!(
                "ERC-165 interface ID for `",
                stringify!(#trait_ident),
                "`. Computed as the XOR of the function selectors of every \
                 method declared on this trait."
            )]
            fn interface_id() -> [u8; 4]
            where
                Self: Sized,
            {
                #(#method_blocks)*
                let mut id = [0u8; 4];
                #(#xor_lines)*
                id
            }
        }
    })
}

/// Build a const expression of type `&'static str` representing the canonical
/// Solidity signature: `name(type1,type2,...)` where `typeN` is each parameter
/// type's `<T as SolEncode>::SOL_NAME`.
///
/// We splice `SolEncode::SOL_NAME` through `const_format::concatcp!` so the
/// full signature is concatenated at compile time. The result is then fed to
/// `const_selector` which is itself a `const fn`.
fn build_signature_concat(name: &str, param_types: &[&syn::Type]) -> TokenStream {
    if param_types.is_empty() {
        let suffix = format!("{}()", name);
        return quote! { #suffix };
    }

    let name_open = format!("{}(", name);
    let close = ")";

    // Build comma-separated `<T as SolEncode>::SOL_NAME` expressions.
    let mut pieces: Vec<TokenStream> = Vec::with_capacity(param_types.len() * 2 + 2);
    pieces.push(quote! { #name_open });
    for (i, ty) in param_types.iter().enumerate() {
        if i > 0 {
            pieces.push(quote! { "," });
        }
        pieces.push(quote! { <#ty as ::pvm_contract_sdk::SolEncode>::SOL_NAME });
    }
    pieces.push(quote! { #close });

    quote! {
        ::pvm_contract_sdk::const_format::concatcp!(#(#pieces),*)
    }
}

/// Parses `#[selector(name = "myFunctionName")]`.
struct SelectorArgs {
    name: String,
}

impl syn::parse::Parse for SelectorArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ident: syn::Ident = input.parse()?;
        if ident != "name" {
            return Err(syn::Error::new(
                ident.span(),
                "expected `name = \"...\"` inside `#[selector(...)]`",
            ));
        }
        let _: Token![=] = input.parse()?;
        let lit: LitStr = input.parse()?;
        Ok(SelectorArgs { name: lit.value() })
    }
}

fn extract_selector_rename(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    let mut found: Option<String> = None;
    for attr in attrs {
        if !attr.path().is_ident("selector") {
            continue;
        }
        if found.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "duplicate `#[selector(...)]` attribute; only one is allowed per method",
            ));
        }
        let args: SelectorArgs = attr.parse_args()?;
        if args.name.is_empty() {
            return Err(syn::Error::new_spanned(
                attr,
                "`#[selector(name = \"\")]`: name must be a non-empty string",
            ));
        }
        found = Some(args.name);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_trait(src: &str) -> ItemTrait {
        syn::parse_str(src).expect("trait parses")
    }

    #[test]
    fn empty_trait_rejected() {
        let t = parse_trait("pub trait Empty {}");
        let err = expand_interface_id(t).unwrap_err().to_string();
        assert!(
            err.contains("at least one method"),
            "Expected empty-trait rejection. Got: {err}"
        );
    }

    #[test]
    fn camel_case_conversion_for_method_names() {
        let t = parse_trait(
            r#"
            pub trait IErc20 {
                fn balance_of(&self, account: Address) -> U256;
            }
        "#,
        );
        let output = expand_interface_id(t).unwrap().to_string();

        // The canonical signature should be built with "balanceOf(", not
        // "balance_of(".
        assert!(
            output.contains("\"balanceOf(\""),
            "Generated code should use camelCase method names.\nGot: {output}"
        );
    }

    #[test]
    fn nullary_method_omits_param_concat() {
        let t = parse_trait(
            r#"
            pub trait Trivial {
                fn ping(&self) -> bool;
            }
        "#,
        );
        let output = expand_interface_id(t).unwrap().to_string();
        // A method with no parameters is encoded as a single string literal,
        // not a `concatcp!` invocation.
        assert!(
            output.contains("const_selector (\"ping()\")"),
            "Nullary methods should use a literal signature.\nGot: {output}"
        );
    }

    #[test]
    fn parametric_method_uses_const_format_concat() {
        let t = parse_trait(
            r#"
            pub trait IErc20 {
                fn transfer(&mut self, to: Address, value: U256) -> Result<bool, Self::Error>;
            }
        "#,
        );
        let output = expand_interface_id(t).unwrap().to_string();
        // We expect const_format::concatcp! to splice in SOL_NAME for each
        // parameter type.
        assert!(
            output.contains("const_format :: concatcp !"),
            "Parametric methods should use concatcp! for the signature.\nGot: {output}"
        );
        assert!(
            output.contains("< Address as :: pvm_contract_sdk :: SolEncode > :: SOL_NAME"),
            "Generated signature should reference SolEncode::SOL_NAME for each param.\nGot: {output}"
        );
    }

    #[test]
    fn selector_rename_attribute() {
        let t = parse_trait(
            r#"
            pub trait Renamed {
                #[selector(name = "customName")]
                fn original_name(&self) -> bool;
            }
        "#,
        );
        let output = expand_interface_id(t).unwrap().to_string();
        assert!(
            output.contains("const_selector (\"customName()\")"),
            "Selector rename should override the camelCase default.\nGot: {output}"
        );
        // The `#[selector(...)]` attribute should be stripped from the
        // output (rustc has no idea what it means).
        assert!(
            !output.contains("# [selector"),
            "#[selector] attribute should be stripped from the emitted trait.\nGot: {output}"
        );
    }

    #[test]
    fn xor_count_matches_method_count() {
        let t = parse_trait(
            r#"
            pub trait Three {
                fn a(&self);
                fn b(&self);
                fn c(&self);
            }
        "#,
        );
        let output = expand_interface_id(t).unwrap().to_string();
        // Three methods → three XOR-accumulate blocks.
        let xor_count = output.matches("id [0] ^=").count();
        assert_eq!(
            xor_count, 3,
            "Expected one XOR-accumulate per method.\nGot: {output}"
        );
    }

    #[test]
    fn appends_interface_id_method() {
        let t = parse_trait(
            r#"
            pub trait HasOne {
                fn a(&self);
            }
        "#,
        );
        let output = expand_interface_id(t).unwrap().to_string();
        assert!(
            output.contains("fn interface_id ()"),
            "Trait should gain an `interface_id()` provided method.\nGot: {output}"
        );
        assert!(
            output.contains("Self : Sized"),
            "`interface_id()` should require `Self: Sized` so dyn traits don't break.\nGot: {output}"
        );
    }
}
