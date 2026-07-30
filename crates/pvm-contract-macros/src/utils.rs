use crate::signature::CustomTypes;
use syn_solidity::ItemFunction;

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

/// The canonical Solidity signature the selector is hashed from. `types` must
/// carry the user-defined types declared alongside the function: a struct
/// parameter hashes as the tuple of its fields (`Point` -> `(uint64,uint64)`),
/// not as its declared name, and the same holds for enums and value types.
pub fn compute_function_signature(item: &ItemFunction, types: &CustomTypes) -> String {
    let params: Vec<String> = item
        .parameters
        .types()
        .map(|ty| types.canonical_name(ty))
        .collect();
    format!("{}({})", item.name(), params.join(","))
}
