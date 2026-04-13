use quote::quote;

#[derive(Debug, Clone, PartialEq)]
pub enum SolType {
    Address,
    Bool,
    Uint(usize),
    Int(usize),
    Bytes(usize),
    DynBytes,
    String,
    Array(Box<SolType>),
    FixedArray(Box<SolType>, usize),
    Tuple(Vec<SolType>),
    Custom(String),
}

impl SolType {
    /// Returns the dynamic/static property only when it can be determined from
    /// syntax alone. Custom types intentionally return `None` and must be
    /// resolved through generated trait expressions.
    pub fn is_dynamic(&self) -> Option<bool> {
        match self {
            SolType::DynBytes | SolType::String | SolType::Array(_) => Some(true),
            SolType::Tuple(types) => {
                let mut any_dynamic = false;
                for ty in types {
                    match ty.is_dynamic() {
                        Some(true) => any_dynamic = true,
                        Some(false) => {}
                        None => return None,
                    }
                }
                Some(any_dynamic)
            }
            SolType::FixedArray(inner, _) => inner.is_dynamic(),
            SolType::Custom(_) => None,
            _ => Some(false),
        }
    }

    /// Returns the ABI head size only when it can be determined from syntax
    /// alone. Custom types intentionally return `None` and must be resolved
    /// through generated trait expressions.
    pub fn head_size(&self) -> Option<usize> {
        match self {
            SolType::FixedArray(inner, size) => match inner.is_dynamic()? {
                true => Some(32),
                false => Some(inner.head_size()? * size),
            },
            SolType::Tuple(types) => match self.is_dynamic()? {
                true => Some(32),
                false => {
                    let mut total = 0usize;
                    for ty in types {
                        total += ty.head_size()?;
                    }
                    Some(total)
                }
            },
            SolType::Custom(_) => None,
            _ => Some(32),
        }
    }

    pub fn canonical_name(&self) -> String {
        match self {
            SolType::Address => "address".to_string(),
            SolType::Bool => "bool".to_string(),
            SolType::Uint(bits) => format!("uint{bits}"),
            SolType::Int(bits) => format!("int{bits}"),
            SolType::Bytes(size) => format!("bytes{size}"),
            SolType::DynBytes => "bytes".to_string(),
            SolType::String => "string".to_string(),
            SolType::Array(inner) => format!("{}[]", inner.canonical_name()),
            SolType::FixedArray(inner, size) => format!("{}[{}]", inner.canonical_name(), size),
            SolType::Tuple(types) => {
                let inner: Vec<_> = types.iter().map(|t| t.canonical_name()).collect();
                format!("({})", inner.join(","))
            }
            SolType::Custom(name) => name.clone(),
        }
    }

    pub fn has_custom_types(&self) -> bool {
        match self {
            SolType::Custom(_) => true,
            SolType::Array(inner) | SolType::FixedArray(inner, _) => inner.has_custom_types(),
            SolType::Tuple(types) => types.iter().any(|t| t.has_custom_types()),
            _ => false,
        }
    }

    pub fn from_rust_type(ty: &syn::Type) -> Option<Self> {
        // Handle Vec<T> and alloc::vec::Vec<T> patterns
        if let syn::Type::Path(type_path) = ty {
            let last_segment = type_path.path.segments.last()?;
            if last_segment.ident == "Vec"
                && let syn::PathArguments::AngleBracketed(args) = &last_segment.arguments
                && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
            {
                return Self::from_rust_type(inner_ty).map(|inner| SolType::Array(Box::new(inner)));
            }
        }

        if let syn::Type::Array(array) = ty
            && let syn::Expr::Lit(expr_lit) = &array.len
            && let syn::Lit::Int(lit_int) = &expr_lit.lit
            && let Ok(size) = lit_int.base10_parse::<usize>()
        {
            let inner = Self::from_rust_type(&array.elem)?;
            // [u8; N] encodes as Solidity bytesN (matching alloy behavior)
            if inner == SolType::Uint(8) {
                return Some(SolType::Bytes(size));
            }
            return Some(SolType::FixedArray(Box::new(inner), size));
        }

        if let syn::Type::Tuple(tuple) = ty {
            let elems = tuple
                .elems
                .iter()
                .map(Self::from_rust_type)
                .collect::<Option<Vec<_>>>()?;
            return Some(SolType::Tuple(elems));
        }

        let type_str = quote!(#ty).to_string().replace(' ', "");

        match type_str.as_str() {
            "Address"
            | "pvm_contract_types::Address"
            | "::pvm_contract_types::Address"
            | "pvm_contract::Address"
            | "::pvm_contract::Address" => Some(SolType::Address),
            "U256" | "ruint::aliases::U256" => Some(SolType::Uint(256)),
            "u256" => Some(SolType::Uint(256)),
            "u128" => Some(SolType::Uint(128)),
            "u64" => Some(SolType::Uint(64)),
            "u32" => Some(SolType::Uint(32)),
            "u16" => Some(SolType::Uint(16)),
            "u8" => Some(SolType::Uint(8)),
            "i128" => Some(SolType::Int(128)),
            "i64" => Some(SolType::Int(64)),
            "i32" => Some(SolType::Int(32)),
            "i16" => Some(SolType::Int(16)),
            "i8" => Some(SolType::Int(8)),
            "bool" => Some(SolType::Bool),
            "String" | "alloc::string::String" => Some(SolType::String),
            "Bytes" | "pvm_contract_types::Bytes" | "::pvm_contract_types::Bytes" => {
                Some(SolType::DynBytes)
            }
            _ => Some(SolType::Custom(type_str)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SolType;

    #[test]
    fn maps_address_newtype_to_solidity_address() {
        let ty: syn::Type = syn::parse_str("Address").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol.canonical_name(), "address");

        let ty: syn::Type = syn::parse_str("pvm_contract_types::Address").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol.canonical_name(), "address");
    }

    #[test]
    fn maps_fixed_arrays_and_tuples() {
        let ty: syn::Type = syn::parse_str("[u64; 4]").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol.canonical_name(), "uint64[4]");

        let ty: syn::Type = syn::parse_str("(Address, u256)").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol.canonical_name(), "(address,uint256)");
    }

    #[test]
    fn maps_custom_paths_to_custom_type() {
        let ty: syn::Type = syn::parse_str("my_crate::Foo").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol.canonical_name(), "my_crate::Foo");
    }

    #[test]
    fn custom_type_becomes_custom_variant() {
        // Proc macro correctly identifies unknown types as Custom
        let ty: syn::Type = syn::parse_str("Count").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert!(
            matches!(sol, SolType::Custom(ref name) if name == "Count"),
            "Type alias should become SolType::Custom, got {:?}",
            sol
        );
    }

    #[test]
    fn selector_resolution_requires_codegen_not_soltype() {
        let sol = SolType::Custom("Count".to_string());
        assert_eq!(sol.canonical_name(), "Count");
    }

    #[test]
    fn is_dynamic_for_dynamic_array() {
        let sol = SolType::Array(Box::new(SolType::Uint(256)));
        assert_eq!(sol.is_dynamic(), Some(true));
    }

    #[test]
    fn is_dynamic_for_string() {
        assert_eq!(SolType::String.is_dynamic(), Some(true));
    }

    #[test]
    fn is_dynamic_for_static_tuple() {
        let sol = SolType::Tuple(vec![SolType::Uint(64), SolType::Bool]);
        assert_eq!(sol.is_dynamic(), Some(false));
    }

    #[test]
    fn is_dynamic_for_tuple_with_dynamic_inner() {
        let sol = SolType::Tuple(vec![SolType::Uint(64), SolType::String]);
        assert_eq!(sol.is_dynamic(), Some(true));
    }

    #[test]
    fn is_dynamic_for_tuple_with_custom_returns_none() {
        let sol = SolType::Tuple(vec![SolType::Uint(64), SolType::Custom("Foo".into())]);
        assert_eq!(sol.is_dynamic(), None);
    }

    #[test]
    fn is_dynamic_for_fixed_array_of_static() {
        let sol = SolType::FixedArray(Box::new(SolType::Uint(256)), 3);
        assert_eq!(sol.is_dynamic(), Some(false));
    }

    #[test]
    fn is_dynamic_for_fixed_array_of_dynamic() {
        let sol = SolType::FixedArray(Box::new(SolType::String), 2);
        assert_eq!(sol.is_dynamic(), Some(true));
    }

    #[test]
    fn head_size_for_primitives() {
        assert_eq!(SolType::Uint(256).head_size(), Some(32));
        assert_eq!(SolType::Address.head_size(), Some(32));
        assert_eq!(SolType::Bool.head_size(), Some(32));
    }

    #[test]
    fn head_size_for_fixed_array_static() {
        let sol = SolType::FixedArray(Box::new(SolType::Uint(256)), 3);
        assert_eq!(sol.head_size(), Some(96));
    }

    #[test]
    fn head_size_for_fixed_array_dynamic() {
        let sol = SolType::FixedArray(Box::new(SolType::String), 2);
        assert_eq!(sol.head_size(), Some(32));
    }

    #[test]
    fn head_size_for_static_tuple() {
        let sol = SolType::Tuple(vec![SolType::Uint(64), SolType::Uint(128)]);
        assert_eq!(sol.head_size(), Some(64));
    }

    #[test]
    fn head_size_for_dynamic_tuple() {
        let sol = SolType::Tuple(vec![SolType::Uint(64), SolType::String]);
        assert_eq!(sol.head_size(), Some(32));
    }

    #[test]
    fn head_size_for_custom_returns_none() {
        assert_eq!(SolType::Custom("Foo".into()).head_size(), None);
    }

    #[test]
    fn has_custom_types_detects_nested() {
        let sol = SolType::Array(Box::new(SolType::Custom("Foo".into())));
        assert!(sol.has_custom_types());

        let sol = SolType::FixedArray(Box::new(SolType::Uint(64)), 3);
        assert!(!sol.has_custom_types());

        let sol = SolType::Tuple(vec![SolType::Bool, SolType::Custom("Bar".into())]);
        assert!(sol.has_custom_types());
    }

    #[test]
    fn maps_vec_to_dynamic_array() {
        let ty: syn::Type = syn::parse_str("Vec<u64>").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol, SolType::Array(Box::new(SolType::Uint(64))));
    }

    #[test]
    fn maps_u8_array_to_bytes_n() {
        let ty: syn::Type = syn::parse_str("[u8; 20]").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol, SolType::Bytes(20));
    }

    #[test]
    fn maps_signed_integers() {
        let ty: syn::Type = syn::parse_str("i32").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol, SolType::Int(32));

        let ty: syn::Type = syn::parse_str("i128").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol, SolType::Int(128));
    }

    #[test]
    fn maps_string_type() {
        let ty: syn::Type = syn::parse_str("String").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol, SolType::String);
    }

    #[test]
    fn maps_bytes_to_dyn_bytes() {
        let ty: syn::Type = syn::parse_str("Bytes").unwrap();
        let sol = SolType::from_rust_type(&ty).unwrap();
        assert_eq!(sol, SolType::DynBytes);
    }
}
