use proc_macro2::TokenStream;
use quote::quote;

use crate::signature::SolType;

fn add_offset(base: &TokenStream, offset: usize) -> TokenStream {
    if offset == 0 {
        base.clone()
    } else {
        quote! { (#base) + #offset }
    }
}

fn generate_decode_expr(
    ty: &SolType,
    data_expr: TokenStream,
    offset_expr: TokenStream,
    use_alloc: bool,
) -> TokenStream {
    match ty {
        SolType::Address => {
            quote! {{
                let mut addr = [0u8; 20];
                addr.copy_from_slice(&#data_expr[#offset_expr + 12..#offset_expr + 32]);
                pvm_contract::Address::from(addr)
            }}
        }
        SolType::Bool => {
            quote! {
                #data_expr[#offset_expr + 31] != 0
            }
        }
        SolType::Uint(8) => {
            quote! {
                #data_expr[#offset_expr + 31]
            }
        }
        SolType::Uint(16) => {
            quote! {
                u16::from_be_bytes([#data_expr[#offset_expr + 30], #data_expr[#offset_expr + 31]])
            }
        }
        SolType::Uint(32) => {
            quote! {
                u32::from_be_bytes(#data_expr[#offset_expr + 28..#offset_expr + 32].try_into().unwrap())
            }
        }
        SolType::Uint(64) => {
            quote! {
                u64::from_be_bytes(#data_expr[#offset_expr + 24..#offset_expr + 32].try_into().unwrap())
            }
        }
        SolType::Uint(128) => {
            quote! {
                u128::from_be_bytes(#data_expr[#offset_expr + 16..#offset_expr + 32].try_into().unwrap())
            }
        }
        SolType::Uint(_) => {
            quote! {
                pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
            }
        }
        SolType::Int(8) => {
            quote! {
                #data_expr[#offset_expr + 31] as i8
            }
        }
        SolType::Int(16) => {
            quote! {
                i16::from_be_bytes([#data_expr[#offset_expr + 30], #data_expr[#offset_expr + 31]])
            }
        }
        SolType::Int(32) => {
            quote! {
                i32::from_be_bytes(#data_expr[#offset_expr + 28..#offset_expr + 32].try_into().unwrap())
            }
        }
        SolType::Int(64) => {
            quote! {
                i64::from_be_bytes(#data_expr[#offset_expr + 24..#offset_expr + 32].try_into().unwrap())
            }
        }
        SolType::Int(128) => {
            quote! {
                i128::from_be_bytes(#data_expr[#offset_expr + 16..#offset_expr + 32].try_into().unwrap())
            }
        }
        SolType::Int(_) => {
            quote! {
                pvm_contract::I256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
            }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            quote! {{
                let mut bytes = [0u8; #size_lit];
                bytes.copy_from_slice(&#data_expr[#offset_expr..#offset_expr + #size_lit]);
                bytes
            }}
        }
        SolType::DynBytes => {
            if use_alloc {
                quote! {{
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
                            .as_limbs()[0] as usize;
                    let length =
                        pvm_contract::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32])
                            .as_limbs()[0] as usize;
                    #data_expr[dyn_offset + 32..dyn_offset + 32 + length].to_vec()
                }}
            } else {
                quote! {{
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
                            .as_limbs()[0] as usize;
                    let length =
                        pvm_contract::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32])
                            .as_limbs()[0] as usize;
                    &#data_expr[dyn_offset + 32..dyn_offset + 32 + length]
                }}
            }
        }
        SolType::String => {
            if use_alloc {
                quote! {{
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
                            .as_limbs()[0] as usize;
                    let length =
                        pvm_contract::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32])
                            .as_limbs()[0] as usize;
                    let bytes = &#data_expr[dyn_offset + 32..dyn_offset + 32 + length];
                    alloc::string::String::from_utf8_lossy(bytes).into_owned()
                }}
            } else {
                quote! {{
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
                            .as_limbs()[0] as usize;
                    let length =
                        pvm_contract::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32])
                            .as_limbs()[0] as usize;
                    let bytes = &#data_expr[dyn_offset + 32..dyn_offset + 32 + length];
                    core::str::from_utf8(bytes).unwrap_or("")
                }}
            }
        }
        SolType::Array(inner) => {
            if use_alloc {
                let elem_size = inner.head_size();
                let inner_decode = generate_decode_expr(
                    inner,
                    quote!(array_data),
                    quote!(i * #elem_size),
                    use_alloc,
                );
                quote! {{
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
                            .as_limbs()[0] as usize;
                    let length =
                        pvm_contract::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32])
                            .as_limbs()[0] as usize;
                    let array_data = &#data_expr[dyn_offset + 32..];
                    let mut result = alloc::vec::Vec::with_capacity(length);
                    for i in 0..length {
                        result.push(#inner_decode);
                    }
                    result
                }}
            } else {
                panic!("Dynamic arrays not supported in no_alloc mode");
            }
        }
        SolType::FixedArray(inner, size) => {
            let elem_size = inner.head_size();
            let elem_decodes: Vec<_> = if inner.is_dynamic() {
                (0..*size)
                    .map(|i| {
                        let elem_offset = i * elem_size;
                        generate_decode_expr(
                            inner,
                            quote!(array_data),
                            quote!(#elem_offset),
                            use_alloc,
                        )
                    })
                    .collect()
            } else {
                (0..*size)
                    .map(|i| {
                        let elem_offset = add_offset(&offset_expr, i * elem_size);
                        generate_decode_expr(inner, data_expr.clone(), elem_offset, use_alloc)
                    })
                    .collect()
            };

            if inner.is_dynamic() {
                quote! {{
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
                            .as_limbs()[0] as usize;
                    let array_data = &#data_expr[dyn_offset..];
                    [#(#elem_decodes),*]
                }}
            } else {
                quote! {
                    [#(#elem_decodes),*]
                }
            }
        }
        SolType::Tuple(types) => {
            let build_tuple = |base_data: TokenStream, base_offset: TokenStream| {
                let mut current_offset = 0usize;
                let elem_decodes: Vec<_> = types
                    .iter()
                    .map(|t| {
                        let decode = generate_decode_expr(
                            t,
                            base_data.clone(),
                            add_offset(&base_offset, current_offset),
                            use_alloc,
                        );
                        current_offset += t.head_size();
                        decode
                    })
                    .collect();
                quote! { (#(#elem_decodes),*) }
            };

            if ty.is_dynamic() {
                let tuple_decode = build_tuple(quote!(tuple_data), quote!(0usize));
                quote! {{
                    let dyn_offset =
                        pvm_contract::U256::from_be_slice(&#data_expr[#offset_expr..#offset_expr + 32])
                            .as_limbs()[0] as usize;
                    let tuple_data = &#data_expr[dyn_offset..];
                    #tuple_decode
                }}
            } else {
                build_tuple(data_expr, offset_expr)
            }
        }
    }
}

pub fn generate_decode(
    ty: &SolType,
    data_expr: TokenStream,
    offset: usize,
    use_alloc: bool,
) -> TokenStream {
    generate_decode_expr(ty, data_expr, quote!(#offset), use_alloc)
}

pub fn generate_decode_params(types: &[SolType], use_alloc: bool) -> Vec<TokenStream> {
    let mut offset = 0;
    types
        .iter()
        .map(|ty| {
            let decode = generate_decode(ty, quote!(input), offset, use_alloc);
            offset += ty.head_size();
            decode
        })
        .collect()
}

pub fn calculate_min_input_size(types: &[SolType]) -> usize {
    types.iter().map(|t| t.head_size()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_dynamic_array_uses_array_base_for_element_offsets() {
        let tokens = generate_decode(
            &SolType::Array(Box::new(SolType::String)),
            quote!(output),
            0,
            true,
        )
        .to_string();
        assert!(tokens.contains("array_data"));
        assert!(!tokens.contains("elem_data"));
    }

    #[test]
    fn decode_dynamic_tuple_resolves_tuple_offset() {
        let tokens = generate_decode(
            &SolType::Tuple(vec![SolType::Uint(64), SolType::String]),
            quote!(output),
            0,
            true,
        )
        .to_string();
        assert!(tokens.contains("tuple_data"));
        assert!(tokens.contains("dyn_offset"));
    }
}
