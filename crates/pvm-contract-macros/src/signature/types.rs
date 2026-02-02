use proc_macro2::TokenStream;
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
        }
    }

    #[allow(dead_code)]
    pub fn rust_type(&self, use_alloc: bool) -> TokenStream {
        match self {
            SolType::Address => quote!(pvm::Address),
            SolType::Bool => quote!(bool),
            SolType::Uint(8) => quote!(u8),
            SolType::Uint(16) => quote!(u16),
            SolType::Uint(32) => quote!(u32),
            SolType::Uint(64) => quote!(u64),
            SolType::Uint(128) => quote!(u128),
            SolType::Uint(_) => quote!(pvm::U256),
            SolType::Int(8) => quote!(i8),
            SolType::Int(16) => quote!(i16),
            SolType::Int(32) => quote!(i32),
            SolType::Int(64) => quote!(i64),
            SolType::Int(128) => quote!(i128),
            SolType::Int(_) => quote!(pvm::I256),
            SolType::Bytes(n) => {
                let n = *n;
                quote!([u8; #n])
            }
            SolType::DynBytes => {
                if use_alloc {
                    quote!(alloc::vec::Vec<u8>)
                } else {
                    quote!(&[u8])
                }
            }
            SolType::String => {
                if use_alloc {
                    quote!(alloc::string::String)
                } else {
                    quote!(&str)
                }
            }
            SolType::Array(inner) => {
                let inner_ty = inner.rust_type(use_alloc);
                if use_alloc {
                    quote!(alloc::vec::Vec<#inner_ty>)
                } else {
                    quote!(&[#inner_ty])
                }
            }
            SolType::FixedArray(inner, size) => {
                let inner_ty = inner.rust_type(use_alloc);
                let size = *size;
                quote!([#inner_ty; #size])
            }
            SolType::Tuple(types) => {
                let inner: Vec<_> = types.iter().map(|t| t.rust_type(use_alloc)).collect();
                quote!((#(#inner),*))
            }
        }
    }

    pub fn is_dynamic(&self) -> bool {
        match self {
            SolType::DynBytes | SolType::String | SolType::Array(_) => true,
            SolType::Tuple(types) => types.iter().any(|t| t.is_dynamic()),
            SolType::FixedArray(inner, _) => inner.is_dynamic(),
            _ => false,
        }
    }

    pub fn head_size(&self) -> usize {
        match self {
            SolType::FixedArray(inner, size) if !inner.is_dynamic() => inner.head_size() * size,
            SolType::Tuple(types) if !self.is_dynamic() => {
                types.iter().map(|t| t.head_size()).sum()
            }
            _ => 32,
        }
    }

    pub fn from_rust_type(ty: &syn::Type) -> Option<Self> {
        let type_str = quote!(#ty).to_string().replace(' ', "");

        match type_str.as_str() {
            "Address" | "pvm_contract::Address" => Some(SolType::Address),
            "U256" | "pvm_contract::U256" => Some(SolType::Uint(256)),
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
            "[u8;20]" => Some(SolType::Bytes(20)),
            "String" | "alloc::string::String" => Some(SolType::String),
            _ => None,
        }
    }
}
