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

pub fn compute_function_signature(item: &ItemFunction) -> String {
    let mut name = format!("{}{}", item.name(), item.call_type());
    if name.rfind(",").is_some_and(|x| x == name.len() - 2) {
        name.remove(name.len() - 2);
    }
    name
}
