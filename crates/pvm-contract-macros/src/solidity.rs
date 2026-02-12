use crate::signature::{FunctionSignature, SolType};

#[derive(Debug, Clone)]
pub struct SolFunction {
    pub name: String,
    pub signature: FunctionSignature,
}

#[derive(Debug, Clone)]
pub struct SolInterface {
    #[allow(dead_code)]
    pub name: String,
    pub functions: Vec<SolFunction>,
}

pub fn parse_solidity_interface(source: &str) -> Result<SolInterface, String> {
    let mut interface_name = String::new();
    let mut functions = Vec::new();

    for line in source.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("interface ") {
            if let Some(end) = rest.find(|c: char| c == '{' || c.is_whitespace()) {
                interface_name = rest[..end].trim().to_string();
            } else {
                interface_name = rest.trim().to_string();
            }
        }

        if line.starts_with("function ")
            && let Some(func) = parse_function_line(line)
        {
            functions.push(func);
        }
    }

    if interface_name.is_empty() {
        return Err("No interface found in Solidity file".to_string());
    }

    Ok(SolInterface {
        name: interface_name,
        functions,
    })
}

fn parse_function_line(line: &str) -> Option<SolFunction> {
    let line = line.strip_prefix("function ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = line.find(')')?;
    let params_str = &line[paren_start + 1..paren_end];

    let inputs = parse_params(params_str);

    let outputs = if let Some(returns_idx) = line.find("returns") {
        let after_returns = &line[returns_idx + 7..];
        if let Some(start) = after_returns.find('(') {
            if let Some(end) = after_returns.find(')') {
                parse_params(&after_returns[start + 1..end])
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let signature = FunctionSignature {
        name: name.clone(),
        inputs,
        outputs,
    };

    Some(SolFunction { name, signature })
}

fn parse_params(params_str: &str) -> Vec<SolType> {
    if params_str.trim().is_empty() {
        return vec![];
    }

    params_str
        .split(',')
        .filter_map(|p| {
            let p = p.trim();
            let type_str = p.split_whitespace().next()?;
            parse_sol_type(type_str)
        })
        .collect()
}

fn parse_sol_type(s: &str) -> Option<SolType> {
    let s = s.trim();

    if s == "address" {
        return Some(SolType::Address);
    }
    if s == "bool" {
        return Some(SolType::Bool);
    }
    if s == "string" {
        return Some(SolType::String);
    }
    if s == "bytes" {
        return Some(SolType::DynBytes);
    }

    if let Some(rest) = s.strip_prefix("uint") {
        let bits: usize = if rest.is_empty() {
            256
        } else {
            rest.parse().ok()?
        };
        return Some(SolType::Uint(bits));
    }

    if let Some(rest) = s.strip_prefix("int") {
        let bits: usize = if rest.is_empty() {
            256
        } else {
            rest.parse().ok()?
        };
        return Some(SolType::Int(bits));
    }

    if let Some(rest) = s.strip_prefix("bytes") {
        let size: usize = rest.parse().ok()?;
        return Some(SolType::Bytes(size));
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
        assert_eq!(iface.name, "MyToken");
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
}
