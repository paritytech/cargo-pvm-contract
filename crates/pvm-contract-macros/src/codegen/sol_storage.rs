use proc_macro2::TokenStream;
use quote::quote;

/// Generate the TokenStream for a single storage layout JSON entry.
///
/// Used by the `#[contract]` slot-field layout generation in `abi_gen.rs`.
pub(super) fn generate_layout_entry(name_str: &str, ty: &syn::Type, slot: u64) -> TokenStream {
    let slot_str = format!("{}", slot);
    quote! {
        {
            let mut entry = ::std::string::String::from("{\"label\":\"");
            entry.push_str(#name_str);
            entry.push_str("\",\"slot\":\"");
            entry.push_str(#slot_str);
            entry.push_str("\",\"type\":\"");
            entry.push_str(&<#ty as ::pvm_contract_sdk::StorageLayoutType>::sol_type_name());
            entry.push_str("\"}");
            entry
        }
    }
}

/// Generate the JSON assembly from a `Vec<String>` of entries.
///
/// Used by the `#[contract]` slot-field layout generation.
pub(super) fn layout_json_from_entries() -> TokenStream {
    quote! {
        let mut json = ::std::string::String::from("{\"storage\":[");
        for (i, entry) in entries.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push_str(entry);
        }
        json.push_str("]}");
        json
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
