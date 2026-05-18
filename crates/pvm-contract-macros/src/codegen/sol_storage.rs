//! `#[storage]` attribute macro: derive [`StorageComponent`] for a user struct
//! whose fields are themselves storage components (typically `Lazy<T>` and
//! `Mapping<K, V>`, but also nested `#[storage]` structs).
//!
//! Generated code is a thin shell over the same auto-numbering const chain
//! used by `#[contract]`:
//!
//! ```ignore
//! #[pvm_contract_sdk::storage]
//! pub struct Erc20 {
//!     total_supply: Lazy<U256>,
//!     balances: Mapping<Address, U256>,
//!     allowances: Mapping<Address, Mapping<Address, U256>>,
//! }
//! ```
//!
//! expands (roughly) to:
//!
//! ```ignore
//! pub struct Erc20 {
//!     total_supply: Lazy<U256>,
//!     balances: Mapping<Address, U256>,
//!     allowances: Mapping<Address, Mapping<Address, U256>>,
//! }
//!
//! impl ::pvm_contract_sdk::StorageComponent for Erc20 {
//!     const SLOTS: u64 =
//!           <Lazy<U256> as StorageComponent>::SLOTS
//!         + <Mapping<Address, U256> as StorageComponent>::SLOTS
//!         + <Mapping<Address, Mapping<Address, U256>> as StorageComponent>::SLOTS;
//!
//!     fn new_at(base: u64, host: ::pvm_contract_sdk::Host) -> Self {
//!         const __OFF_total_supply: u64 = 0;
//!         const __OFF_balances: u64 =
//!             __OFF_total_supply + <Lazy<U256> as StorageComponent>::SLOTS;
//!         const __OFF_allowances: u64 =
//!             __OFF_balances + <Mapping<Address, U256> as StorageComponent>::SLOTS;
//!         Erc20 {
//!             total_supply: <Lazy<U256> as StorageComponent>::new_at(
//!                 base + __OFF_total_supply, host.clone()),
//!             balances: <_ as StorageComponent>::new_at(
//!                 base + __OFF_balances, host.clone()),
//!             allowances: <_ as StorageComponent>::new_at(
//!                 base + __OFF_allowances, host),
//!         }
//!     }
//! }
//! ```
//!
//! Notes:
//! - Tuple and unit structs are rejected; only named-field structs make sense
//!   here because slot ordering is meaningful.
//! - `#[storage]` does NOT yet support pinning individual offsets via
//!   `#[slot(N)]`. The expectation is that embedded storage structs are
//!   declared in their natural field order; if the user wants a specific
//!   layout they can declare the leaf fields directly on the contract struct.
//! - The macro must be placed *before* any field uses the type, but doesn't
//!   need to be in the same module — the generated trait impl lives next to
//!   the struct, so it's visible wherever the struct is.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Fields, ItemStruct};

pub fn expand_storage_struct(input: ItemStruct) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let named = match &input.fields {
        Fields::Named(named) => named,
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[storage] requires a struct with named fields. Unit and tuple structs are not supported.",
            ));
        }
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[storage] requires a struct with named fields. Tuple structs are not supported.",
            ));
        }
    };

    if named.named.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "#[storage] requires at least one storage field.",
        ));
    }

    let field_names: Vec<&syn::Ident> = named
        .named
        .iter()
        .map(|f| f.ident.as_ref().expect("named fields"))
        .collect();
    let field_types: Vec<&syn::Type> = named.named.iter().map(|f| &f.ty).collect();
    let field_cfgs: Vec<Vec<&syn::Attribute>> = named
        .named
        .iter()
        .map(|f| {
            f.attrs
                .iter()
                .filter(|a| a.path().is_ident("cfg"))
                .collect()
        })
        .collect();

    // The SLOTS const sums every field's contribution.
    let slot_terms: Vec<TokenStream> = field_types
        .iter()
        .map(|ty| quote! { <#ty as ::pvm_contract_sdk::StorageComponent>::SLOTS })
        .collect();
    let slots_expr = if slot_terms.is_empty() {
        quote! { 0u64 }
    } else {
        quote! { #(#slot_terms)+* }
    };

    // Per-field offset const chain (relative to base).
    let offset_consts: Vec<TokenStream> = field_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let const_ident = format_ident!("__pvm_storage_offset_{}", name);
            let cfgs = &field_cfgs[i];
            if i == 0 {
                quote! {
                    #(#cfgs)*
                    #[allow(non_upper_case_globals)]
                    const #const_ident: u64 = 0;
                }
            } else {
                let prev_name = field_names[i - 1];
                let prev_ty = field_types[i - 1];
                let prev_const = format_ident!("__pvm_storage_offset_{}", prev_name);
                quote! {
                    #(#cfgs)*
                    #[allow(non_upper_case_globals)]
                    const #const_ident: u64 = #prev_const
                        + <#prev_ty as ::pvm_contract_sdk::StorageComponent>::SLOTS;
                }
            }
        })
        .collect();

    let field_inits: Vec<TokenStream> = field_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ty = field_types[i];
            let cfgs = &field_cfgs[i];
            let const_ident = format_ident!("__pvm_storage_offset_{}", name);
            quote! {
                #(#cfgs)*
                #name: <#ty as ::pvm_contract_sdk::StorageComponent>::new_at(
                    base + #const_ident,
                    host.clone(),
                )
            }
        })
        .collect();

    // The user's struct, unchanged.
    let user_struct = &input;

    Ok(quote! {
        #user_struct

        impl #impl_generics ::pvm_contract_sdk::StorageComponent
            for #struct_name #ty_generics
        #where_clause
        {
            const SLOTS: u64 = #slots_expr;

            fn new_at(base: u64, host: ::pvm_contract_sdk::Host) -> Self {
                #(#offset_consts)*
                #struct_name {
                    #(#field_inits),*
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ItemStruct {
        syn::parse_str(src).expect("input parses")
    }

    #[test]
    fn generates_storage_component_impl() {
        let input = parse(
            r#"
            pub struct Erc20 {
                total_supply: Lazy<U256>,
                balances: Mapping<Address, U256>,
            }
        "#,
        );
        let output = expand_storage_struct(input).unwrap().to_string();

        // The original struct is preserved.
        assert!(
            output.contains("pub struct Erc20"),
            "struct should be preserved: {output}"
        );

        // SLOTS sums each field's SLOTS.
        assert!(
            output.contains("const SLOTS : u64 = < Lazy < U256 > as :: pvm_contract_sdk :: StorageComponent > :: SLOTS + < Mapping < Address , U256 > as :: pvm_contract_sdk :: StorageComponent > :: SLOTS"),
            "SLOTS const should sum field SLOTS. Got: {output}"
        );

        // First field's offset is 0.
        assert!(
            output.contains("const __pvm_storage_offset_total_supply : u64 = 0 ;"),
            "first offset should be 0: {output}"
        );

        // Each field's slot const is base + offset.
        assert!(
            output.contains("base + __pvm_storage_offset_total_supply"),
            "field init should reference its offset const: {output}"
        );
    }

    #[test]
    fn rejects_tuple_struct() {
        let input = parse("pub struct T(u32, u32);");
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(
            err.contains("Tuple structs are not supported"),
            "Got: {err}"
        );
    }

    #[test]
    fn rejects_unit_struct() {
        let input = parse("pub struct U;");
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(
            err.contains("Unit and tuple structs are not supported"),
            "Got: {err}"
        );
    }

    #[test]
    fn rejects_empty_named_struct() {
        let input = parse("pub struct E {}");
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(err.contains("at least one storage field"), "Got: {err}");
    }

    #[test]
    fn supports_generics() {
        let input = parse(
            r#"
            pub struct Container<T> {
                value: Lazy<T>,
            }
        "#,
        );
        let output = expand_storage_struct(input).unwrap().to_string();
        // The impl picks up the generics.
        assert!(
            output
                .contains("impl < T > :: pvm_contract_sdk :: StorageComponent for Container < T >"),
            "should propagate generics: {output}"
        );
    }
}
