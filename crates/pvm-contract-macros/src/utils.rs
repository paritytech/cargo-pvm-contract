use core::num::NonZeroU16;

use syn_solidity::{ItemFunction, SolPath, Spanned, Type};

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

pub fn compute_function_signature_substituting_enums(
    item: &ItemFunction,
    is_enum: impl Fn(&SolPath) -> bool,
) -> String {
    let mut param_types: Vec<Type> = item.parameters.types().cloned().collect();
    for ty in &mut param_types {
        substitute_enum_type(ty, &is_enum);
    }
    let call_type = Type::Tuple(param_types.into_iter().collect());
    let mut name = format!("{}{}", item.name(), call_type);
    if name.rfind(",").is_some_and(|x| x == name.len() - 2) {
        name.remove(name.len() - 2);
    }
    name
}

fn substitute_enum_type(ty: &mut Type, is_enum: &impl Fn(&SolPath) -> bool) {
    match ty {
        Type::Custom(path) if is_enum(path) => {
            let span = path.span();
            *ty = Type::Uint(span, NonZeroU16::new(8));
        }
        Type::Array(array) => substitute_enum_type(&mut array.ty, is_enum),
        Type::Tuple(tuple) => {
            for inner in tuple.types.iter_mut() {
                substitute_enum_type(inner, is_enum);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fn(src: &str) -> ItemFunction {
        syn::parse_str(src).expect("solidity function parses")
    }

    fn is_my_enum(path: &SolPath) -> bool {
        path.last() == "MyEnum"
    }

    #[test]
    fn enum_param_renders_as_uint8() {
        let func = parse_fn("function setColor(MyEnum color) external;");
        let sig = compute_function_signature_substituting_enums(&func, is_my_enum);
        assert_eq!(sig, "setColor(uint8)");
    }

    #[test]
    fn non_enum_signature_is_unchanged() {
        let func = parse_fn("function transfer(address to, uint256 amount) external;");
        let sig = compute_function_signature_substituting_enums(&func, |_| false);
        assert_eq!(sig, "transfer(address,uint256)");
    }

    #[test]
    fn enum_nested_in_array_and_tuple_renders_as_uint8() {
        let func = parse_fn("function g(MyEnum[2] a, (MyEnum, uint256) b) external;");
        let sig = compute_function_signature_substituting_enums(&func, is_my_enum);
        assert_eq!(sig, "g(uint8[2],(uint8,uint256))");
    }

    #[test]
    fn non_enum_custom_type_is_not_substituted() {
        let func = parse_fn("function h(SomeStruct s) external;");
        let sig = compute_function_signature_substituting_enums(&func, is_my_enum);
        assert_eq!(sig, "h(SomeStruct)");
    }
}
