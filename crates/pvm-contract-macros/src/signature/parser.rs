use super::types::SolType;

#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub name: String,
    pub inputs: Vec<SolType>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub outputs: Vec<SolType>,
}

impl FunctionSignature {
    pub fn parse(sig: &str) -> Result<Self, String> {
        let sig = sig.trim();

        let (main_part, outputs) = if let Some(idx) = sig.find(" returns ") {
            let (main, ret) = sig.split_at(idx);
            let ret = ret.trim_start_matches(" returns ").trim();
            (main, Self::parse_type_list(ret)?)
        } else if let Some(idx) = sig.find("returns(") {
            let (main, ret) = sig.split_at(idx);
            let ret = ret.trim_start_matches("returns").trim();
            (main.trim(), Self::parse_type_list(ret)?)
        } else {
            (sig, vec![])
        };

        let open_paren = main_part
            .find('(')
            .ok_or_else(|| format!("Missing '(' in signature: {sig}"))?;
        let close_paren = main_part
            .rfind(')')
            .ok_or_else(|| format!("Missing ')' in signature: {sig}"))?;

        let name = main_part[..open_paren].trim().to_string();
        if name.is_empty() {
            return Err("Empty function name".to_string());
        }

        let params_str = &main_part[open_paren + 1..close_paren];
        let inputs = Self::parse_params(params_str)?;

        Ok(FunctionSignature {
            name,
            inputs,
            outputs,
        })
    }

    fn parse_type_list(s: &str) -> Result<Vec<SolType>, String> {
        let s = s.trim();
        if !s.starts_with('(') || !s.ends_with(')') {
            return Err(format!("Type list must be wrapped in parentheses: {s}"));
        }
        Self::parse_params(&s[1..s.len() - 1])
    }

    fn parse_params(params_str: &str) -> Result<Vec<SolType>, String> {
        let params_str = params_str.trim();
        if params_str.is_empty() {
            return Ok(vec![]);
        }

        let mut types = vec![];
        let mut depth = 0;
        let mut current = String::new();

        for ch in params_str.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    let ty = Self::parse_single_type(current.trim())?;
                    types.push(ty);
                    current.clear();
                }
                _ => current.push(ch),
            }
        }

        if !current.trim().is_empty() {
            let ty = Self::parse_single_type(current.trim())?;
            types.push(ty);
        }

        Ok(types)
    }

    pub(crate) fn parse_single_type(s: &str) -> Result<SolType, String> {
        let s = s.trim();

        if s.starts_with('(') {
            let close = Self::find_matching_paren(s, 0)?;
            let tuple_inner = &s[1..close];
            let tuple_types = Self::parse_params(tuple_inner)?;

            let rest = &s[close + 1..];
            return Self::apply_array_suffixes(SolType::Tuple(tuple_types), rest);
        }

        if let Some(bracket_start) = s.find('[') {
            let base_type_str = &s[..bracket_start];
            let base_type = Self::parse_base_type(base_type_str)?;
            return Self::apply_array_suffixes(base_type, &s[bracket_start..]);
        }

        Self::parse_base_type(s)
    }

    fn parse_base_type(s: &str) -> Result<SolType, String> {
        let s = s.trim();

        match s {
            "address" => Ok(SolType::Address),
            "bool" => Ok(SolType::Bool),
            "string" => Ok(SolType::String),
            "bytes" => Ok(SolType::DynBytes),
            _ if s.starts_with("uint") => {
                let bits: usize = s[4..].parse().unwrap_or(256);
                if bits == 0 || bits > 256 || !bits.is_multiple_of(8) {
                    return Err(format!("Invalid uint size: {bits}"));
                }
                Ok(SolType::Uint(bits))
            }
            _ if s.starts_with("int") => {
                let bits: usize = s[3..].parse().unwrap_or(256);
                if bits == 0 || bits > 256 || !bits.is_multiple_of(8) {
                    return Err(format!("Invalid int size: {bits}"));
                }
                Ok(SolType::Int(bits))
            }
            _ if s.starts_with("bytes") => {
                let size: usize = s[5..]
                    .parse()
                    .map_err(|_| format!("Invalid bytes size: {s}"))?;
                if size == 0 || size > 32 {
                    return Err(format!("Invalid bytes size: {size}"));
                }
                Ok(SolType::Bytes(size))
            }
            _ => Err(format!("Unknown type: {s}")),
        }
    }

    fn apply_array_suffixes(base: SolType, suffix: &str) -> Result<SolType, String> {
        let suffix = suffix.trim();
        if suffix.is_empty() {
            return Ok(base);
        }

        if !suffix.starts_with('[') {
            return Err(format!("Expected '[' but found: {suffix}"));
        }

        let close = suffix
            .find(']')
            .ok_or_else(|| format!("Missing ']' in: {suffix}"))?;
        let size_str = &suffix[1..close];
        let rest = &suffix[close + 1..];

        let array_type = if size_str.is_empty() {
            SolType::Array(Box::new(base))
        } else {
            let size: usize = size_str
                .parse()
                .map_err(|_| format!("Invalid array size: {size_str}"))?;
            SolType::FixedArray(Box::new(base), size)
        };

        Self::apply_array_suffixes(array_type, rest)
    }

    fn find_matching_paren(s: &str, start: usize) -> Result<usize, String> {
        let mut depth = 0;
        for (i, ch) in s[start..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(start + i);
                    }
                }
                _ => {}
            }
        }
        Err(format!("Unmatched parentheses in: {s}"))
    }

    pub fn canonical_signature(&self) -> String {
        let params: Vec<String> = self.inputs.iter().map(|t| t.canonical_name()).collect();
        format!("{}({})", self.name, params.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let sig = FunctionSignature::parse("transfer(address,uint256)").unwrap();
        assert_eq!(sig.name, "transfer");
        assert_eq!(sig.inputs.len(), 2);
        assert_eq!(sig.inputs[0], SolType::Address);
        assert_eq!(sig.inputs[1], SolType::Uint(256));
        assert!(sig.outputs.is_empty());
    }

    #[test]
    fn test_parse_with_returns() {
        let sig = FunctionSignature::parse("balanceOf(address) returns (uint256)").unwrap();
        assert_eq!(sig.name, "balanceOf");
        assert_eq!(sig.inputs.len(), 1);
        assert_eq!(sig.outputs.len(), 1);
        assert_eq!(sig.outputs[0], SolType::Uint(256));
    }

    #[test]
    fn test_parse_no_params() {
        let sig = FunctionSignature::parse("totalSupply() returns (uint256)").unwrap();
        assert_eq!(sig.name, "totalSupply");
        assert!(sig.inputs.is_empty());
        assert_eq!(sig.outputs.len(), 1);
    }

    #[test]
    fn test_parse_array() {
        let sig = FunctionSignature::parse("batchTransfer(address[],uint256[])").unwrap();
        assert_eq!(sig.inputs.len(), 2);
        assert!(matches!(sig.inputs[0], SolType::Array(_)));
        assert!(matches!(sig.inputs[1], SolType::Array(_)));
    }

    #[test]
    fn test_parse_fixed_array() {
        let sig = FunctionSignature::parse("setCoords(uint256[3])").unwrap();
        assert_eq!(sig.inputs.len(), 1);
        assert!(matches!(sig.inputs[0], SolType::FixedArray(_, 3)));
    }

    #[test]
    fn test_parse_tuple() {
        let sig = FunctionSignature::parse("foo((address,uint256))").unwrap();
        assert_eq!(sig.inputs.len(), 1);
        match &sig.inputs[0] {
            SolType::Tuple(types) => {
                assert_eq!(types.len(), 2);
                assert_eq!(types[0], SolType::Address);
                assert_eq!(types[1], SolType::Uint(256));
            }
            _ => panic!("Expected tuple"),
        }
    }

    #[test]
    fn test_canonical_signature() {
        let sig = FunctionSignature::parse("transfer(address,uint256)").unwrap();
        assert_eq!(sig.canonical_signature(), "transfer(address,uint256)");
    }

    #[test]
    fn test_parse_nested_tuple() {
        let sig = FunctionSignature::parse("foo((uint256,address),bool)").unwrap();
        assert_eq!(sig.inputs.len(), 2);
        match &sig.inputs[0] {
            SolType::Tuple(types) => {
                assert_eq!(types.len(), 2);
                assert_eq!(types[0], SolType::Uint(256));
                assert_eq!(types[1], SolType::Address);
            }
            _ => panic!("Expected tuple"),
        }
        assert_eq!(sig.inputs[1], SolType::Bool);
    }

    #[test]
    fn test_parse_empty_returns() {
        let sig = FunctionSignature::parse("foo(uint256) returns ()").unwrap();
        assert_eq!(sig.inputs.len(), 1);
        assert!(sig.outputs.is_empty());
    }

    #[test]
    fn test_parse_multiple_returns() {
        let sig = FunctionSignature::parse("foo(uint256) returns (bool,address,uint256)").unwrap();
        assert_eq!(sig.outputs.len(), 3);
        assert_eq!(sig.outputs[0], SolType::Bool);
        assert_eq!(sig.outputs[1], SolType::Address);
        assert_eq!(sig.outputs[2], SolType::Uint(256));
    }

    #[test]
    fn test_parse_bytes_types() {
        let sig = FunctionSignature::parse("foo(bytes32,bytes)").unwrap();
        assert_eq!(sig.inputs[0], SolType::Bytes(32));
        assert_eq!(sig.inputs[1], SolType::DynBytes);
    }

    #[test]
    fn test_parse_signed_integers() {
        let sig = FunctionSignature::parse("foo(int8,int128)").unwrap();
        assert_eq!(sig.inputs[0], SolType::Int(8));
        assert_eq!(sig.inputs[1], SolType::Int(128));
    }

    #[test]
    fn test_parse_string_type() {
        let sig = FunctionSignature::parse("foo(string) returns (string)").unwrap();
        assert_eq!(sig.inputs[0], SolType::String);
        assert_eq!(sig.outputs[0], SolType::String);
    }

    #[test]
    fn test_parse_unknown_type_errors() {
        let result = FunctionSignature::parse("foo(foobar)");
        assert!(result.is_err());
    }
}
