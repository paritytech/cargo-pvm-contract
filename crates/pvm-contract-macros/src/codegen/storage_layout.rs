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
/// For `Lazy<T>` / `Mapping<K, V>` (recognised syntactically) this emits a
/// single `entries.push(StorageLayoutEntry { … })` with the type name resolved
/// through `<T as SolEncode>::SOL_NAME`. For any other type the field is
/// treated as an embedded `#[storage]` sub-struct and dispatched through
/// [`pvm_contract_sdk::StorageLayoutEmit::emit_entries`], which recursively
/// flattens its leaves into the same `entries` Vec, prefixing labels with the
/// field path (`erc20.total_supply`, `metadata.name`, …) per solc convention.
///
/// `slot_expr` is a `u64` expression (literal or `base + __pvm_storage_offset_*`
/// const). `prefix_expr` is a `&str` expression: `""` at the top of a
/// `#[contract]`, the inherited `name_prefix` argument inside a `#[storage]`
/// `emit_entries` body.
///
/// Used by `#[contract]`'s `__storage_layout_json` (top-level) and `#[storage]`'s
/// `__storage_layout_entries` (sub-storage).
pub(super) fn generate_layout_emit(
    field_name_str: &str,
    ty: &syn::Type,
    slot_expr: TokenStream,
    offset_expr: TokenStream,
    prefix_expr: TokenStream,
) -> TokenStream {
    if is_layout_leaf(ty) {
        let ty_name_expr = sol_storage_type_name(ty);
        quote! {
            entries.push(::pvm_contract_sdk::StorageLayoutEntry {
                label: ::pvm_contract_sdk::join_label(#prefix_expr, #field_name_str),
                slot: {
                    let slot_value: u64 = #slot_expr;
                    ::std::format!("{}", slot_value)
                },
                offset: #offset_expr,
                ty: #ty_name_expr,
            });
        }
    } else {
        // Caller is expected to have a `&mut Vec<StorageLayoutEntry>` binding
        // in scope named `entries`. `entries.push(...)` works against either an
        // owned Vec or a `&mut Vec`, while passing `entries` straight into the
        // trait call auto-reborrows when it's already a `&mut`.
        quote! {
            <#ty as ::pvm_contract_sdk::StorageLayoutEmit>::emit_entries(
                #slot_expr,
                &::pvm_contract_sdk::join_label(#prefix_expr, #field_name_str),
                entries,
            );
        }
    }
}

/// Whether the type's layout entry is a single inlined leaf (`Lazy<T>` or
/// `Mapping<K, V>`) rather than something that should recurse through
/// [`StorageLayoutEmit`].
fn is_layout_leaf(ty: &syn::Type) -> bool {
    matches!(wrapper_and_type_args(ty), Some((name, args)) if {
        (name == "Lazy" && args.len() == 1) || (name == "Mapping" && args.len() == 2)
    })
}

/// Build a `String`-valued token expression that names the Solidity storage
/// type for a storage field's Rust type. Unwraps `Lazy<T>` and recurses into
/// `Mapping<K, V>` syntactically; everything else is named via
/// `<T as StorageTypeName>::NAME`.
///
/// `StorageTypeName` (in `pvm-contract-types`) has no blanket impl — every
/// storage-eligible type provides an explicit one (primitives in
/// `storage_codec.rs` / `alloc_types.rs`), so primitives and
/// `#[derive(SolStorage)]` value-shaped structs work out of the box. The
/// `#[storage]` / `#[derive(SolStorage)]` derives emit a `StorageTypeName`
/// impl for the target struct returning the Rust ident — this is what makes
/// `Mapping<K, MyStorageStruct>` produce `"mapping(K, MyStorageStruct)"`
/// in the layout JSON. Map keys (`K`) are also resolved through
/// `StorageTypeName`, so any key type that has a name is acceptable.
fn sol_storage_type_name(ty: &syn::Type) -> TokenStream {
    if let Some((wrapper, args)) = wrapper_and_type_args(ty) {
        match (wrapper.as_str(), args.as_slice()) {
            ("Lazy", [inner]) => {
                return sol_storage_type_name(inner);
            }
            ("Mapping", [k, v]) => {
                let v_expr = sol_storage_type_name(v);
                return quote! {
                    ::std::format!(
                        "mapping({},{})",
                        <#k as ::pvm_contract_sdk::StorageTypeName>::name(),
                        #v_expr,
                    )
                };
            }
            _ => {}
        }
    }
    quote! {
        <#ty as ::pvm_contract_sdk::StorageTypeName>::name()
    }
}

/// If `ty` is a path type whose final segment is `Lazy` or `Mapping`, return
/// the segment name and the type-position generic arguments. Matches on the
/// last segment's ident only, so `Lazy<T>`, `pvm_storage::Lazy<T>`, and
/// `pvm_contract_sdk::Lazy<T>` all resolve.
///
/// Returns `None` for any other type shape, which falls through to the
/// `SolEncode::SOL_NAME` leaf path.
fn wrapper_and_type_args(ty: &syn::Type) -> Option<(String, Vec<&syn::Type>)> {
    let path = match ty {
        syn::Type::Path(tp) if tp.qself.is_none() => &tp.path,
        _ => return None,
    };
    let last = path.segments.last()?;
    let name = last.ident.to_string();
    if name != "Lazy" && name != "Mapping" {
        return None;
    }
    let args = match &last.arguments {
        syn::PathArguments::AngleBracketed(a) => a,
        _ => return None,
    };
    let type_args: Vec<&syn::Type> = args
        .args
        .iter()
        .filter_map(|a| match a {
            syn::GenericArgument::Type(t) => Some(t),
            _ => None,
        })
        .collect();
    Some((name, type_args))
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
