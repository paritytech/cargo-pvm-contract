use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// One field that participates in an auto-numbered storage slot chain.
///
/// Used by [`slot_chain_consts`] to emit a sequence of compile-time consts
/// whose values walk `prev + <PrevTy as StorageComponent>::SLOTS`. The first
/// const evaluates to `0`; downstream code adds an explicit base (e.g.
/// `base + #const_ident`) when the chain is relative to a runtime offset.
pub(super) struct ChainField<'a> {
    pub name: &'a syn::Ident,
    pub ty: &'a syn::Type,
    pub cfg_attrs: &'a [syn::Attribute],
}

/// Build a chain of `const <alone_prefix><name>: bool = ...;` items that
/// each tell whether the corresponding field is alone in its storage slot
/// — i.e. no sibling field shares the same slot index.
///
/// The const evaluates by comparing the field's `LayoutStep.slot` against
/// the adjacent neighbours' `.slot`. A field with no left neighbour or no
/// right neighbour skips that comparison; a single isolated field is
/// trivially `true`. The result feeds the `alone: bool` argument of
/// [`StorageComponent::new_at`](pvm_contract_sdk::StorageComponent::new_at)
/// so sub-word `Lazy<T>` can skip the read-modify-write SLOAD when the slot
/// has no sub-word neighbour.
///
/// `slot_idents` are the `LayoutStep` const idents produced by
/// [`slot_chain_consts`] (one per field, in the same order); the generated
/// comparisons reference them directly, so there is no prefix string to keep
/// in sync between the two builders. Per-field `#[cfg]` attributes are
/// propagated.
pub(super) fn alone_chain_consts(
    alone_prefix: &str,
    slot_idents: &[syn::Ident],
    fields: &[ChainField],
) -> Vec<TokenStream> {
    fields
        .iter()
        .enumerate()
        .map(|(i, sf)| {
            let alone_ident = format_ident!("{}{}", alone_prefix, sf.name);
            let cfgs = sf.cfg_attrs;
            let cur_slot = &slot_idents[i];
            // Comparison against the previous field (if any).
            let prev_check = if i == 0 {
                quote! { true }
            } else {
                let prev_slot = &slot_idents[i - 1];
                quote! { #cur_slot.slot != #prev_slot.slot }
            };
            // Comparison against the next field (if any).
            let next_check = if i + 1 == fields.len() {
                quote! { true }
            } else {
                let next_slot = &slot_idents[i + 1];
                quote! { #cur_slot.slot != #next_slot.slot }
            };
            quote! {
                #(#cfgs)*
                #[allow(non_upper_case_globals)]
                const #alone_ident: bool = #prev_check && #next_check;
            }
        })
        .collect()
}

/// Build a chain of `const <prefix><name>: ::pvm_contract_sdk::LayoutStep
/// = ::pvm_contract_sdk::layout_step(prev, PACKED_BYTES, SLOTS);` items for
/// the supplied fields. First entry seeds from
/// [`LayoutStep::FIRST`](pvm_contract_sdk::LayoutStep::FIRST); each
/// subsequent entry chains off the previous step. Per-field `#[cfg]`
/// attributes are propagated so cfg-disabled fields disappear from the
/// chain at use sites.
///
/// Each `LayoutStep` carries the field's placement (`.slot`, `.offset`)
/// and the next field's chain seed (`.next_slot`, `.next_space`). Callers
/// read `.slot` + `.offset` to construct the field, and pass the entire
/// step as the previous step for the next field.
///
/// Shared by `#[contract]` (top-level struct fields) and `#[storage]`
/// (sub-storage struct fields, with the chain re-rooted at `base`).
///
/// Returns the generated const items together with their idents (in field
/// order) so callers — notably [`alone_chain_consts`] — can reference the
/// consts by value instead of reconstructing their names from `prefix`.
pub(super) fn slot_chain_consts(
    prefix: &str,
    fields: &[ChainField],
) -> (Vec<TokenStream>, Vec<syn::Ident>) {
    let idents: Vec<syn::Ident> = fields
        .iter()
        .map(|sf| format_ident!("{}{}", prefix, sf.name))
        .collect();
    let items = fields
        .iter()
        .enumerate()
        .map(|(i, sf)| {
            let const_ident = &idents[i];
            let cfgs = sf.cfg_attrs;
            let ty = sf.ty;
            let prev_expr = if i == 0 {
                quote! { ::pvm_contract_sdk::LayoutStep::FIRST }
            } else {
                let prev_const = &idents[i - 1];
                quote! { #prev_const }
            };
            quote! {
                #(#cfgs)*
                #[allow(non_upper_case_globals)]
                const #const_ident: ::pvm_contract_sdk::LayoutStep =
                    ::pvm_contract_sdk::layout_step(
                        #prev_expr,
                        <#ty as ::pvm_contract_sdk::StorageComponent>::PACKED_BYTES,
                        <#ty as ::pvm_contract_sdk::StorageComponent>::SLOTS,
                    );
            }
        })
        .collect();
    (items, idents)
}

/// Generate the TokenStream that pushes storage-layout entries for one field
/// into the local `entries` Vec.
///
/// Every field — `Lazy<T>`, `Mapping<K, V>`, or an embedded `#[storage]`
/// sub-struct — dispatches uniformly through
/// [`pvm_contract_sdk::StorageLayoutEmit::emit_entries`]. Leaf types push a
/// single entry; sub-structs recursively flatten their own leaves into the
/// same `entries` Vec, prefixing labels with the field path
/// (`erc20.total_supply`, `metadata.name`, …) per solc convention. There is
/// no syntactic type-name special-casing: `<#ty as StorageLayoutEmit>` is the
/// single source of truth for both the entry's `type` string and its layout,
/// so adding a storage component is a pure trait-impl task.
///
/// `slot_expr` is a `u64` expression (literal or `base + __pvm_storage_offset_*`
/// const); `offset_expr` is a `u8` expression (the packed byte offset, `0` for
/// full-slot fields). `prefix_expr` is a `&str` expression: `""` at the top of
/// a `#[contract]`, the inherited `name_prefix` argument inside a `#[storage]`
/// `emit_entries` body.
///
/// Used by `#[contract]`'s `__storage_layout_json` (top-level) and `#[storage]`'s
/// `emit_entries` (sub-storage).
pub(super) fn generate_layout_emit(
    field_name_str: &str,
    ty: &syn::Type,
    slot_expr: TokenStream,
    offset_expr: TokenStream,
    prefix_expr: TokenStream,
) -> TokenStream {
    // Caller is expected to have a `&mut Vec<StorageLayoutEntry>` binding in
    // scope named `entries`; passing it straight into the trait call
    // auto-reborrows.
    quote! {
        <#ty as ::pvm_contract_sdk::StorageLayoutEmit>::emit_entries(
            #slot_expr,
            #offset_expr,
            &::pvm_contract_sdk::join_label(#prefix_expr, #field_name_str),
            entries,
        );
    }
}

/// Extract the `#[slot(N)]` attribute value from a field, if present.
/// Returns `None` when the field has no `#[slot]` attribute.
pub(super) fn extract_optional_slot_attr(field: &syn::Field) -> syn::Result<Option<u64>> {
    let mut found: Option<&syn::Attribute> = None;
    for attr in &field.attrs {
        if attr.path().is_ident("slot") {
            if found.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "duplicate #[slot] attribute; each field must have exactly one",
                ));
            }
            found = Some(attr);
        }
    }
    let Some(attr) = found else {
        return Ok(None);
    };
    let slot: syn::LitInt = attr.parse_args()?;
    Ok(Some(slot.base10_parse::<u64>()?))
}
