use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Fields};

pub fn expand_sol_storage(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "SolStorage does not support generic structs",
        ));
    }

    let name = &input.ident;

    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        syn::Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "SolStorage can only be derived for structs",
            ));
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "SolStorage cannot be derived for unions",
            ));
        }
    };

    let named_fields = match fields {
        Fields::Named(named) => named,
        _ => {
            return Err(syn::Error::new_spanned(
                &input,
                "SolStorage requires named fields",
            ));
        }
    };

    // Parse #[slot(N)] attributes from each field
    let mut field_entries = Vec::new();
    for field in &named_fields.named {
        let field_name = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "SolStorage fields must be named"))?;
        let field_ty = &field.ty;
        let slot = extract_slot_attr(field)?;
        field_entries.push((field_name.clone(), field_ty.clone(), slot));
    }

    // Check for duplicate slot numbers
    for (i, (name_a, _, slot_a)) in field_entries.iter().enumerate() {
        for (name_b, _, slot_b) in &field_entries[i + 1..] {
            if slot_a == slot_b {
                return Err(syn::Error::new_spanned(
                    &named_fields.named,
                    format!(
                        "duplicate slot {}: fields `{}` and `{}` use the same slot number",
                        slot_a, name_a, name_b
                    ),
                ));
            }
        }
    }

    // Generate the SolStorage trait impl
    let field_inits: Vec<TokenStream> = field_entries
        .iter()
        .map(|(name, ty, slot)| {
            let slot_lit = *slot;
            quote! {
                #name: <#ty>::new(::pvm_contract_sdk::StorageKey::from_slot(#slot_lit))
            }
        })
        .collect();

    let storage_impl = quote! {
        impl ::pvm_contract_sdk::SolStorage for #name {
            fn __pvm_storage() -> Self {
                Self {
                    #(#field_inits),*
                }
            }
        }
    };

    // Generate __storage_layout_json() behind cfg(abi-gen)
    let layout_entries: Vec<TokenStream> = field_entries
        .iter()
        .map(|(name, ty, slot)| {
            let name_str = name.to_string();
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
        })
        .collect();

    let layout_fn = quote! {
        #[cfg(feature = "abi-gen")]
        impl #name {
            #[doc(hidden)]
            pub fn __storage_layout_json() -> ::std::string::String {
                let entries: ::std::vec::Vec<::std::string::String> = ::std::vec![
                    #(#layout_entries),*
                ];
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
    };

    Ok(quote! {
        #storage_impl
        #layout_fn
    })
}

/// Extract the `#[slot(N)]` attribute value from a field.
/// Errors if the field has zero or more than one `#[slot]` attribute.
fn extract_slot_attr(field: &syn::Field) -> syn::Result<u64> {
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
    let attr = found.ok_or_else(|| {
        syn::Error::new_spanned(field, "SolStorage fields must have a #[slot(N)] attribute")
    })?;
    let slot: syn::LitInt = attr.parse_args()?;
    slot.base10_parse::<u64>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::DeriveInput;

    #[test]
    fn rejects_duplicate_slot_numbers() {
        let input: DeriveInput = syn::parse_str(
            r#"
            struct Storage {
                #[slot(0)]
                a: Lazy<U256>,
                #[slot(0)]
                b: Lazy<U256>,
            }
            "#,
        )
        .unwrap();

        let result = expand_sol_storage(input);
        assert!(result.is_err(), "should reject duplicate slot numbers");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate slot 0"),
            "error should mention the duplicate slot. Got: {err}"
        );
    }

    #[test]
    fn rejects_duplicate_slot_attribute_on_field() {
        let input: DeriveInput = syn::parse_str(
            r#"
            struct Storage {
                #[slot(0)]
                #[slot(1)]
                a: Lazy<U256>,
            }
            "#,
        )
        .unwrap();

        let result = expand_sol_storage(input);
        assert!(
            result.is_err(),
            "should reject fields with multiple #[slot] attributes"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate #[slot]"),
            "error should mention the duplicate attribute. Got: {err}"
        );
    }

    #[test]
    fn rejects_missing_slot_attribute() {
        let input: DeriveInput = syn::parse_str(
            r#"
            struct Storage {
                a: Lazy<U256>,
            }
            "#,
        )
        .unwrap();

        let result = expand_sol_storage(input);
        assert!(result.is_err(), "should reject fields without #[slot(N)]");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("#[slot(N)]"),
            "error should mention the missing attribute. Got: {err}"
        );
    }

    #[test]
    fn rejects_generic_structs() {
        let input: DeriveInput = syn::parse_str(
            r#"
            struct Storage<T> {
                #[slot(0)]
                value: Lazy<T>,
            }
            "#,
        )
        .unwrap();

        let result = expand_sol_storage(input);
        assert!(result.is_err(), "should reject generic structs");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("does not support generic"),
            "error should mention generics. Got: {err}"
        );
    }

    #[test]
    fn rejects_enums() {
        let input: DeriveInput = syn::parse_str(
            r#"
            enum NotAStruct {
                A,
                B,
            }
            "#,
        )
        .unwrap();

        let result = expand_sol_storage(input);
        assert!(result.is_err(), "should reject enums");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only be derived for structs"),
            "error should mention struct requirement. Got: {err}"
        );
    }

    #[test]
    fn rejects_malformed_slot_values() {
        let cases = [
            ("non-integer", "struct S { #[slot(foo)] a: Lazy<U256> }"),
            ("negative", "struct S { #[slot(-1)] a: Lazy<U256> }"),
            ("empty", "struct S { #[slot()] a: Lazy<U256> }"),
            (
                "overflow",
                "struct S { #[slot(99999999999999999999999)] a: Lazy<U256> }",
            ),
        ];
        for (label, src) in cases {
            let input: DeriveInput = syn::parse_str(src).unwrap();
            assert!(
                expand_sol_storage(input).is_err(),
                "should reject malformed slot: {label}"
            );
        }
    }

    #[test]
    fn accepts_valid_storage_struct() {
        let input: DeriveInput = syn::parse_str(
            r#"
            struct Storage {
                #[slot(0)]
                total_supply: Lazy<U256>,
                #[slot(1)]
                balances: Mapping<Address, U256>,
            }
            "#,
        )
        .unwrap();

        let result = expand_sol_storage(input);
        assert!(
            result.is_ok(),
            "should accept valid storage struct: {:?}",
            result.err()
        );
    }
}
