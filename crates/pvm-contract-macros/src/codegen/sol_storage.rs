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
//!                 base + __OFF_allowances, host.clone()),
//!         }
//!         // Every field — including the last — receives `host.clone()`.
//!         // `Host` is a ZST on riscv64 and a cheap `Rc` clone on host
//!         // targets, so cloning per-field has no measurable cost.
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

use super::contract::find_derive_clone;
use super::storage_layout::{ChainField, generate_layout_emit, slot_chain_consts};

pub fn expand_storage_struct(input: ItemStruct) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Same rationale as `#[contract]` storage structs: `#[storage]` components
    // are embedded in a contract storage tree where the borrow checker enforces
    // view-vs-mutating access (`&self` vs `&mut self`). A `Clone` impl would
    // let a view method clone the sub-component and obtain a fresh `&mut`,
    // bypassing the gate.
    if let Some(bad) = find_derive_clone(&input.attrs) {
        return Err(syn::Error::new_spanned(
            bad,
            "#[storage] structs must not derive `Clone`; the mutation gate \
             (`&self` vs `&mut self`) relies on the storage component being \
             `!Clone` to prevent view methods from smuggling out a `&mut`",
        ));
    }

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

    // #[storage] fields are always auto-numbered (no `#[slot(N)]` support yet),
    // so a #[cfg]-disabled field would break the offset const chain *and*
    // silently shift every later field's slot. Reject up front; users who need
    // conditional fields should declare them directly on the contract struct
    // with `#[slot(N)]`.
    for f in &named.named {
        if let Some(cfg) = f.attrs.iter().find(|a| a.path().is_ident("cfg")) {
            return Err(syn::Error::new_spanned(
                cfg,
                "#[cfg] is not supported on #[storage] fields: it would shift \
                 the on-chain slot numbers of every field after it, producing a \
                 different storage layout per feature combination.",
            ));
        }
    }

    let field_names: Vec<&syn::Ident> = named
        .named
        .iter()
        .map(|f| f.ident.as_ref().expect("named fields"))
        .collect();
    let field_types: Vec<&syn::Type> = named.named.iter().map(|f| &f.ty).collect();

    // The SLOTS const sums every field's contribution.
    let slots_expr = {
        let terms: Vec<TokenStream> = field_types
            .iter()
            .map(|ty| quote! { <#ty as ::pvm_contract_sdk::StorageComponent>::SLOTS })
            .collect();
        quote! { #(#terms)+* }
    };

    // Per-field offset const chain (relative to base). Shared with `#[contract]`
    // via `slot_chain_consts` so both macros agree on the chain shape.
    //
    // `cfg_attrs: &[]` is safe here because the `#[cfg]`-on-field check above
    // rejects every cfg-attributed field with a compile error. If that
    // restriction is ever relaxed, this must be replaced with `&f.cfg_attrs`
    // so the chain consts are gated consistently with their fields.
    let chain_fields: Vec<ChainField<'_>> = field_names
        .iter()
        .zip(field_types.iter())
        .map(|(name, ty)| ChainField {
            name,
            ty,
            cfg_attrs: &[],
        })
        .collect();
    let offset_consts = slot_chain_consts("__pvm_storage_offset_", &chain_fields);

    let field_inits: Vec<TokenStream> = field_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ty = field_types[i];
            let const_ident = format_ident!("__pvm_storage_offset_{}", name);
            quote! {
                #name: <#ty as ::pvm_contract_sdk::StorageComponent>::new_at(
                    base + #const_ident.slot,
                    #const_ident.offset,
                    host.clone(),
                )
            }
        })
        .collect();

    // Per-field layout-emit code: leaves push directly, embedded `#[storage]`
    // sub-structs recurse via `StorageLayoutEmit::emit_entries`.
    let layout_emits: Vec<TokenStream> = field_names
        .iter()
        .zip(field_types.iter())
        .map(|(name, ty)| {
            let const_ident = format_ident!("__pvm_storage_offset_{}", name);
            let slot_expr = quote! { base + #const_ident.slot };
            let offset_expr = quote! { #const_ident.offset };
            generate_layout_emit(
                &name.to_string(),
                ty,
                slot_expr,
                offset_expr,
                quote! { name_prefix },
            )
        })
        .collect();

    // Reuse the offset chain inside the layout helper. Each `emit_entries`
    // call rebuilds it locally so the consts stay in fn scope and the helper
    // doesn't have to track an external chain.
    let offset_consts_for_layout = offset_consts.clone();

    // The user's struct, unchanged.
    let user_struct = &input;

    Ok(quote! {
        #user_struct

        impl #impl_generics ::pvm_contract_sdk::StorageComponent
            for #struct_name #ty_generics
        #where_clause
        {
            const SLOTS: u64 = #slots_expr;

            // Embedded `#[storage]` sub-structs always start a fresh slot and
            // never pack with neighbouring contract fields. Matches solc —
            // packing applies inside the sub-struct, never across its outer
            // boundary.
            const PACKED_BYTES: usize = 32;

            fn new_at(base: u64, offset: u8, host: ::pvm_contract_sdk::Host) -> Self {
                debug_assert_eq!(offset, 0,
                    "#[storage] sub-struct always full-slot; offset must be 0");
                let _ = offset;
                #(#offset_consts)*
                #struct_name {
                    #(#field_inits),*
                }
            }
        }

        #[cfg(feature = "abi-gen")]
        impl #impl_generics ::pvm_contract_sdk::StorageLayoutEmit
            for #struct_name #ty_generics
        #where_clause
        {
            // `entries: &mut Vec<...>` matches the call shape in
            // `__storage_layout_json` and in the macro-generated leaf-push /
            // trait-recursion code: leaves use auto-deref for `entries.push(...)`,
            // recursions pass `entries` directly to reborrow.
            fn emit_entries(
                base: u64,
                name_prefix: &str,
                entries: &mut ::std::vec::Vec<::pvm_contract_sdk::StorageLayoutEntry>,
            ) {
                #(#offset_consts_for_layout)*
                #(#layout_emits)*
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

        // First field seeds from LayoutStep::FIRST via layout_step.
        assert!(
            output.contains(
                "const __pvm_storage_offset_total_supply : :: pvm_contract_sdk :: LayoutStep = :: pvm_contract_sdk :: layout_step (:: pvm_contract_sdk :: LayoutStep :: FIRST ,"
            ),
            "first offset should seed from LayoutStep::FIRST: {output}"
        );

        // Each field's slot const is base + step.slot, offset is step.offset.
        assert!(
            output.contains("base + __pvm_storage_offset_total_supply . slot"),
            "field init should derive its slot from base + step.slot: {output}"
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
    fn rejects_derive_clone() {
        let input = parse(
            r#"
            #[derive(Clone)]
            pub struct S {
                a: Lazy<U256>,
            }
        "#,
        );
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(err.contains("must not derive `Clone`"), "Got: {err}");
    }

    #[test]
    fn rejects_cfg_on_storage_field() {
        let input = parse(
            r#"
            pub struct S {
                #[cfg(feature = "extra")]
                a: Lazy<U256>,
                b: Lazy<U256>,
            }
        "#,
        );
        let err = expand_storage_struct(input).unwrap_err().to_string();
        assert!(err.contains("#[cfg] is not supported"), "Got: {err}");
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
