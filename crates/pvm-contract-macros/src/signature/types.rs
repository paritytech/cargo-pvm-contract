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
    pub fn canonical_name(&self) -> String {
        match self {
            SolType::Address => "address".to_string(),
            SolType::Bool => "bool".to_string(),
            SolType::Uint(bits) => format!("uint{}", bits),
            SolType::Int(bits) => format!("int{}", bits),
            SolType::Bytes(size) => format!("bytes{}", size),
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

    pub fn is_dynamic(&self) -> bool {
        match self {
            SolType::DynBytes | SolType::String | SolType::Array(_) => true,
            SolType::Tuple(types) => types.iter().any(|t| t.is_dynamic()),
            SolType::FixedArray(inner, _) => inner.is_dynamic(),
            SolType::Custom(_) => false,
            _ => false,
        }
    }

    pub fn head_size(&self) -> usize {
        match self {
            SolType::FixedArray(inner, size) if !inner.is_dynamic() => inner.head_size() * size,
            SolType::Tuple(types) if !self.is_dynamic() => {
                types.iter().map(|t| t.head_size()).sum()
            }
            SolType::Custom(_) => 0,
            _ => 32,
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

        let type_str = quote!(#ty).to_string().replace(' ', "");

        match type_str.as_str() {
            "Address"
            | "pvm_contract_types::Address"
            | "::pvm_contract_types::Address"
            | "pvm_contract::Address"
            | "::pvm_contract::Address" => Some(SolType::Address),
            "[u8;20]" => Some(SolType::Address),
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
            "[u8;32]" => Some(SolType::Bytes(32)),
            "String" | "alloc::string::String" => Some(SolType::String),
            _ => None,
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
}
