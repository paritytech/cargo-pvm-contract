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
pub(super) fn slot_chain_consts(prefix: &str, fields: &[ChainField]) -> Vec<TokenStream> {
    fields
        .iter()
        .enumerate()
        .map(|(i, sf)| {
            let const_ident = format_ident!("{}{}", prefix, sf.name);
            let cfgs = sf.cfg_attrs;
            let ty = sf.ty;
            let prev_expr = if i == 0 {
                quote! { ::pvm_contract_sdk::LayoutStep::FIRST }
            } else {
                let prev = &fields[i - 1];
                let prev_const = format_ident!("{}{}", prefix, prev.name);
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
        .collect()
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
                // The walker tracks `offset` as the big-endian start index of
                // the field's bytes (distance from the most-significant byte).
                // solc's storageLayout counts `offset` from the least-significant
                // byte, so convert: `solc_offset = 32 - high - size`, where
                // `size` is the field's packed width. Full-slot types
                // (`PACKED_BYTES == 32`, `offset == 0`) map to `0` unchanged.
                // Right-alignment holds for every value type in solc storage
                // (integers, bool, address, and `bytesN`), so this one formula
                // covers all leaves. The internal RMW window in `Lazy::set/get`
                // keeps using the unconverted big-endian `offset`.
                offset: {
                    let __high: u8 = #offset_expr;
                    32u8 - __high
                        - <#ty as ::pvm_contract_sdk::StorageComponent>::PACKED_BYTES as u8
                },
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

/// Whether the type's layout entry is a single inlined leaf (`Lazy<T>`,
/// `Mapping<K, V>`, or `StorageVec<T>`) rather than something that should
/// recurse through [`StorageLayoutEmit`].
fn is_layout_leaf(ty: &syn::Type) -> bool {
    matches!(wrapper_and_type_args(ty), Some((name, args)) if {
        (name == "Lazy" && args.len() == 1)
            || (name == "Mapping" && args.len() == 2)
            || (name == "StorageVec" && args.len() == 1)
    })
}

/// Build a `String`-valued token expression that names the Solidity storage
/// type for a storage field's Rust type. Unwraps `Lazy<T>`, recurses into
/// `Mapping<K, V>` and `StorageVec<T>` (`T[]`) syntactically; everything else
/// is named via `<T as SolEncode>::SOL_NAME`.
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
                        <#k as ::pvm_contract_sdk::SolEncode>::SOL_NAME,
                        #v_expr,
                    )
                };
            }
            // `StorageVec<T>` is Solidity's `T[]`. Recurse on the element type
            // so `StorageVec<StorageVec<U256>>` resolves to `uint256[][]` and
            // `Mapping<K, StorageVec<T>>` nests correctly.
            ("StorageVec", [inner]) => {
                let inner_expr = sol_storage_type_name(inner);
                return quote! {
                    ::std::format!("{}[]", #inner_expr)
                };
            }
            _ => {}
        }
    }
    quote! {
        ::std::string::String::from(<#ty as ::pvm_contract_sdk::SolEncode>::SOL_NAME)
    }
}

/// If `ty` is a path type whose final segment is `Lazy`, `Mapping`, or
/// `StorageVec`, return the segment name and the type-position generic
/// arguments. Matches on the last segment's ident only, so `Lazy<T>`,
/// `pvm_storage::Lazy<T>`, and `pvm_contract_sdk::Lazy<T>` all resolve.
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
    if name != "Lazy" && name != "Mapping" && name != "StorageVec" {
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
