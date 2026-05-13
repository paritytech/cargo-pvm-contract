use proc_macro2::TokenStream;
use quote::quote;

/// Generate the TokenStream that constructs a `StorageLayoutEntry` for one field.
///
/// `slot_expr` is a const-evaluable `u64` expression — either a literal (for
/// explicit `#[slot(N)]`) or a reference to a chained `__pvm_storage_slot_*`
/// const (for auto-numbered fields).
///
/// Used by the `#[contract]` slot-field layout generation in `abi_gen.rs`.
pub(super) fn generate_layout_entry(
    name_str: &str,
    ty: &syn::Type,
    slot_expr: TokenStream,
) -> TokenStream {
    quote! {
        ::pvm_contract_sdk::StorageLayoutEntry {
            label: ::std::string::String::from(#name_str),
            slot: {
                // The slot expr is `u64`. Stringify it at runtime since auto-numbered
                // slots are only known after const evaluation of the chain.
                let slot_value: u64 = #slot_expr;
                ::std::format!("{}", slot_value)
            },
            ty: <#ty as ::pvm_contract_sdk::StorageLayoutType>::sol_type_name(),
        }
    }
}

/// Generate the JSON serialization from a `Vec<StorageLayoutEntry>`.
///
/// Used by the `#[contract]` slot-field layout generation.
pub(super) fn layout_json_from_entries() -> TokenStream {
    quote! {
        let layout = ::pvm_contract_sdk::StorageLayout { storage: entries };
        ::pvm_contract_sdk::storage_layout_to_json(&layout)
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
