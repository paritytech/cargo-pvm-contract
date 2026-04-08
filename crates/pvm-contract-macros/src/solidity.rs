use crate::signature::FunctionSignature;

#[derive(Debug, Clone)]
pub struct SolFunction {
    pub name: String,
    pub signature: FunctionSignature,
}

#[derive(Debug, Clone)]
pub struct SolInterface {
    pub functions: Vec<SolFunction>,
}

pub fn parse_solidity_interface(source: &str) -> Result<SolInterface, String> {
    let mut interface_name = String::new();
    let mut functions = Vec::new();
    let mut pending: Option<String> = None;

    for line in source.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("interface ") {
            if let Some(end) = rest.find(|c: char| c == '{' || c.is_whitespace()) {
                interface_name = rest[..end].trim().to_string();
            } else {
                interface_name = rest.trim().to_string();
            }
        }

        if let Some(ref mut acc) = pending {
            acc.push(' ');
            acc.push_str(line);
            if has_balanced_parens(acc) {
                if let Some(func) = parse_function_line(acc) {
                    functions.push(func);
                }
                pending = None;
            }
        } else if line.starts_with("function ") {
            if has_balanced_parens(line) {
                if let Some(func) = parse_function_line(line) {
                    functions.push(func);
                }
            } else {
                pending = Some(line.to_string());
            }
        }
    }

    if interface_name.is_empty() {
        return Err("No interface found in Solidity file".to_string());
    }

    Ok(SolInterface { functions })
}

/// Check if all opening parens have matching closing parens.
fn has_balanced_parens(s: &str) -> bool {
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn parse_function_line(line: &str) -> Option<SolFunction> {
    let line = line.strip_prefix("function ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();
    let params_end = find_matching_paren(line, paren_start)?;
    let params_str = &line[paren_start + 1..params_end];

    let params = canonicalize_params(params_str)?;
    let mut signature_str = format!("{}({})", name, params.join(","));

    if let Some(returns_idx) = line.find("returns") {
        let after_returns = &line[returns_idx + 7..];
        let returns_start_rel = after_returns.find('(')?;
        let returns_start = returns_idx + 7 + returns_start_rel;
        let returns_end = find_matching_paren(line, returns_start)?;
        let returns_str = &line[returns_start + 1..returns_end];
        let returns = canonicalize_params(returns_str)?;
        signature_str.push_str(" returns (");
        signature_str.push_str(&returns.join(","));
        signature_str.push(')');
    }

    let signature = FunctionSignature::parse(&signature_str).ok()?;

    Some(SolFunction { name, signature })
}

fn canonicalize_params(params_str: &str) -> Option<Vec<String>> {
    split_top_level(params_str)
        .into_iter()
        .map(|param| canonicalize_param(&param))
        .collect()
}

fn split_top_level(params_str: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut depth_paren = 0;
    let mut depth_bracket = 0;
    let mut current = String::new();

    for ch in params_str.chars() {
        match ch {
            '(' => {
                depth_paren += 1;
                current.push(ch);
            }
            ')' => {
                depth_paren -= 1;
                current.push(ch);
            }
            '[' => {
                depth_bracket += 1;
                current.push(ch);
            }
            ']' => {
                depth_bracket -= 1;
                current.push(ch);
            }
            ',' if depth_paren == 0 && depth_bracket == 0 => {
                if !current.trim().is_empty() {
                    params.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        params.push(current.trim().to_string());
    }

    params
}

fn canonicalize_param(param: &str) -> Option<String> {
    let param = param.trim();
    if param.is_empty() {
        return None;
    }

    if param.starts_with('(') {
        let close = find_matching_paren(param, 0)?;
        let tuple_inner = &param[1..close];
        let tuple_types = canonicalize_params(tuple_inner)?;

        let mut ty = format!("({})", tuple_types.join(","));
        let suffix = param[close + 1..]
            .chars()
            .take_while(|c| *c == '[' || *c == ']' || c.is_ascii_digit())
            .collect::<String>();
        ty.push_str(&suffix);
        return Some(ty);
    }

    let mut ty = String::new();
    for ch in param.chars() {
        if ch.is_whitespace() {
            break;
        }
        ty.push(ch);
    }

    if ty.is_empty() {
        return None;
    }

    Some(ty)
}

fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_interface() {
        let source = r#"
            interface MyToken {
                function totalSupply() external view returns (uint256);
                function balanceOf(address account) external view returns (uint256);
                function transfer(address to, uint256 amount) external;
            }
        "#;

        let iface = parse_solidity_interface(source).unwrap();
        assert_eq!(iface.functions.len(), 3);

        assert_eq!(iface.functions[0].name, "totalSupply");
        assert_eq!(iface.functions[1].name, "balanceOf");
        assert_eq!(iface.functions[2].name, "transfer");
    }

    #[test]
    fn test_snake_case() {
        assert_eq!(to_snake_case("totalSupply"), "total_supply");
        assert_eq!(to_snake_case("balanceOf"), "balance_of");
        assert_eq!(to_snake_case("transfer"), "transfer");
    }

    #[test]
    fn test_parse_tuple_and_fixed_array_signature() {
        let line = "function foo((address,uint256) payload, uint256[3] coords) external";
        let function = parse_function_line(line).unwrap();

        assert_eq!(
            function.signature.canonical_signature(),
            "foo((address,uint256),uint256[3])"
        );
    }

    #[test]
    fn test_no_interface_keyword_errors() {
        let source = r#"
            contract MyToken {
                function totalSupply() external view returns (uint256);
            }
        "#;
        let result = parse_solidity_interface(source);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No interface found in Solidity file");
    }

    #[test]
    fn test_empty_interface() {
        let source = r#"
            interface IEmpty {
            }
        "#;
        let iface = parse_solidity_interface(source).unwrap();
        assert!(iface.functions.is_empty());
    }

    #[test]
    fn test_comment_lines_skipped() {
        let source = r#"
            // SPDX-License-Identifier: MIT
            interface IToken {
                // This is a comment
                function totalSupply() external view returns (uint256);
                /* block comment */
                function transfer(address to, uint256 amount) external;
            }
        "#;
        let iface = parse_solidity_interface(source).unwrap();
        assert_eq!(iface.functions.len(), 2);
        assert_eq!(iface.functions[0].name, "totalSupply");
        assert_eq!(iface.functions[1].name, "transfer");
    }

    #[test]
    fn test_multiline_function_declaration() {
        let source = r#"
            interface IToken {
                function transfer(
                    address to,
                    uint256 amount
                ) external returns (bool);
                function totalSupply() external view returns (uint256);
            }
        "#;
        let iface = parse_solidity_interface(source).unwrap();
        assert_eq!(iface.functions.len(), 2);
        assert_eq!(iface.functions[0].name, "transfer");
        assert_eq!(iface.functions[1].name, "totalSupply");
    }

    #[test]
    fn test_interface_brace_on_next_line() {
        let source = r#"
            interface IToken
            {
                function totalSupply() external view returns (uint256);
            }
        "#;
        let iface = parse_solidity_interface(source).unwrap();
        assert_eq!(iface.functions.len(), 1);
        assert_eq!(iface.functions[0].name, "totalSupply");
    }
}
