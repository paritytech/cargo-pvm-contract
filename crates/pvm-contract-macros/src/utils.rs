use proc_macro2::TokenStream;
use quote::quote;
use syn_solidity::ItemFunction;

/// Build the const-eval expression for a method's canonical Solidity signature,
/// e.g. `concatcp!("transfer(", <Address>::SOL_NAME, ",", <U256>::SOL_NAME, ")")`.
///
/// Each parameter's ABI name is resolved from its `SolEncode::SOL_NAME` at
/// const-eval, so custom types work without the macro knowing their ABI name.
/// Shared by the dispatch selector codegen and `#[interface_id]` so both derive
/// selector text identically.
pub fn build_method_signature_expr(sol_name: &str, param_types: &[syn::Type]) -> TokenStream {
    let prefix = format!("{sol_name}(");
    let mut parts: Vec<TokenStream> = vec![quote! { #prefix }];
    for (i, ty) in param_types.iter().enumerate() {
        if i > 0 {
            parts.push(quote! { "," });
        }
        parts.push(quote! { <#ty as ::pvm_contract_sdk::SolEncode>::SOL_NAME });
    }
    parts.push(quote! { ")" });
    quote! { ::pvm_contract_sdk::const_format::concatcp!(#(#parts),*) }
}

/// Convert snake_case to camelCase, the default Solidity name for a Rust method.
/// The leading segment stays lower-case (`balance_of` becomes `balanceOf`).
pub fn to_camel_case(snake: &str) -> String {
    let mut result = String::new();
    let mut next_upper = false;
    for (i, c) in snake.chars().enumerate() {
        if c == '_' {
            next_upper = true;
        } else if i == 0 {
            result.push(c);
        } else if next_upper {
            result.push(c.to_ascii_uppercase());
            next_upper = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// A Solidity identifier is `[a-zA-Z_$][a-zA-Z0-9_$]*`.
pub fn is_valid_solidity_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Reject `name` unless it is a Solidity identifier, attaching the diagnostic to
/// `spanned`. The single source of the "Invalid Solidity identifier" message,
/// shared by the `#[selector(name)]` and `#[method(rename)]` rename paths.
pub fn validate_sol_identifier(name: &str, spanned: impl quote::ToTokens) -> syn::Result<()> {
    if is_valid_solidity_identifier(name) {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            spanned,
            format!(
                "Invalid Solidity identifier `{name}`. \
                 Must match [a-zA-Z_$][a-zA-Z0-9_$]*"
            ),
        ))
    }
}

/// Read a `#[selector(name = "...")]` rename off a method's attribute list.
///
/// This is the canonical spelling of the Solidity-name override, shared by the
/// `#[interface_id]` trait surface and the inherent `#[method]` path (where
/// `#[method(rename = "...")]` remains a supported alias). Accepts both the bare
/// `#[selector(...)]` and prefixed `#[pvm_contract_sdk::selector(...)]` forms.
/// The name is validated as a Solidity identifier.
pub fn extract_selector_rename(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    let mut found: Option<String> = None;
    for attr in attrs {
        if attr.path().segments.last().map(|s| s.ident.to_string()) != Some("selector".to_string())
        {
            continue;
        }

        if found.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "duplicate `#[selector(name = \"...\")]`; a method may carry at most one",
            ));
        }

        let mut name: Option<syn::LitStr> = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                name = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error("unknown `#[selector]` argument; expected `name = \"...\"`"))
            }
        })?;

        let Some(lit) = name else {
            return Err(syn::Error::new_spanned(
                attr,
                "`#[selector]` requires `name = \"...\"`",
            ));
        };
        let value = lit.value();
        validate_sol_identifier(&value, &lit)?;
        found = Some(value);
    }
    Ok(found)
}

pub fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

pub fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut post_lower = false;
    for (i, c) in s.chars().enumerate() {
        if c == '_' {
            post_lower = true;
        } else {
            let c = if post_lower || i == 0 {
                c.to_ascii_uppercase()
            } else {
                c
            };
            post_lower = false;
            result.push(c);
        }
    }
    result
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

pub fn compute_function_signature(item: &ItemFunction) -> String {
    let mut name = format!("{}{}", item.name(), item.call_type());
    if name.rfind(",").is_some_and(|x| x == name.len() - 2) {
        name.remove(name.len() - 2);
    }
    name
}
