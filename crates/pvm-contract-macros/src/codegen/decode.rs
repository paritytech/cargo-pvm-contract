use proc_macro2::TokenStream;
use quote::quote;

use crate::signature::SolType;

pub fn generate_decode(
    ty: &SolType,
    data_expr: TokenStream,
    offset: usize,
    use_alloc: bool,
) -> TokenStream {
    let offset_lit = offset;

    match ty {
        SolType::Address => {
            quote! {{
                let mut addr = [0u8; 20];
                addr.copy_from_slice(&#data_expr[#offset_lit + 12..#offset_lit + 32]);
                addr
            }}
        }
        SolType::Bool => {
            quote! {
                #data_expr[#offset_lit + 31] != 0
            }
        }
        SolType::Uint(8) => {
            quote! {
                #data_expr[#offset_lit + 31]
            }
        }
        SolType::Uint(16) => {
            quote! {
                u16::from_be_bytes([#data_expr[#offset_lit + 30], #data_expr[#offset_lit + 31]])
            }
        }
        SolType::Uint(32) => {
            quote! {
                u32::from_be_bytes(#data_expr[#offset_lit + 28..#offset_lit + 32].try_into().unwrap())
            }
        }
        SolType::Uint(64) => {
            quote! {
                u64::from_be_bytes(#data_expr[#offset_lit + 24..#offset_lit + 32].try_into().unwrap())
            }
        }
        SolType::Uint(128) => {
            quote! {
                u128::from_be_bytes(#data_expr[#offset_lit + 16..#offset_lit + 32].try_into().unwrap())
            }
        }
        SolType::Uint(_) => {
            quote! {
                ruint::aliases::U256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32])
            }
        }
        SolType::Int(8) => {
            quote! {
                #data_expr[#offset_lit + 31] as i8
            }
        }
        SolType::Int(16) => {
            quote! {
                i16::from_be_bytes([#data_expr[#offset_lit + 30], #data_expr[#offset_lit + 31]])
            }
        }
        SolType::Int(32) => {
            quote! {
                i32::from_be_bytes(#data_expr[#offset_lit + 28..#offset_lit + 32].try_into().unwrap())
            }
        }
        SolType::Int(64) => {
            quote! {
                i64::from_be_bytes(#data_expr[#offset_lit + 24..#offset_lit + 32].try_into().unwrap())
            }
        }
        SolType::Int(128) => {
            quote! {
                i128::from_be_bytes(#data_expr[#offset_lit + 16..#offset_lit + 32].try_into().unwrap())
            }
        }
        SolType::Int(_) => {
            quote! {
                ruint::aliases::I256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32])
            }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            quote! {{
                let mut bytes = [0u8; #size_lit];
                bytes.copy_from_slice(&#data_expr[#offset_lit..#offset_lit + #size_lit]);
                bytes
            }}
        }
        SolType::DynBytes => {
            if use_alloc {
                quote! {{
                    let dyn_offset = ruint::aliases::U256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32]).as_limbs()[0] as usize;
                    let length = ruint::aliases::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
                    #data_expr[dyn_offset + 32..dyn_offset + 32 + length].to_vec()
                }}
            } else {
                quote! {{
                    let dyn_offset = ruint::aliases::U256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32]).as_limbs()[0] as usize;
                    let length = ruint::aliases::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
                    &#data_expr[dyn_offset + 32..dyn_offset + 32 + length]
                }}
            }
        }
        SolType::String => {
            if use_alloc {
                quote! {{
                    let dyn_offset = ruint::aliases::U256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32]).as_limbs()[0] as usize;
                    let length = ruint::aliases::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
                    let bytes = &#data_expr[dyn_offset + 32..dyn_offset + 32 + length];
                    alloc::string::String::from_utf8_lossy(bytes).into_owned()
                }}
            } else {
                quote! {{
                    let dyn_offset = ruint::aliases::U256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32]).as_limbs()[0] as usize;
                    let length = ruint::aliases::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
                    let bytes = &#data_expr[dyn_offset + 32..dyn_offset + 32 + length];
                    core::str::from_utf8(bytes).unwrap_or("")
                }}
            }
        }
        SolType::Array(inner) => {
            if use_alloc {
                let inner_decode =
                    generate_decode_array_element(inner, quote!(elem_data), use_alloc);
                let elem_size = inner.head_size();
                quote! {{
                    let dyn_offset = ruint::aliases::U256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32]).as_limbs()[0] as usize;
                    let length = ruint::aliases::U256::from_be_slice(&#data_expr[dyn_offset..dyn_offset + 32]).as_limbs()[0] as usize;
                    let array_data = &#data_expr[dyn_offset + 32..];
                    let mut result = alloc::vec::Vec::with_capacity(length);
                    for i in 0..length {
                        let elem_data = &array_data[i * #elem_size..];
                        result.push(#inner_decode);
                    }
                    result
                }}
            } else {
                panic!("Dynamic arrays not supported in no_alloc mode");
            }
        }
        SolType::FixedArray(inner, size) => {
            let _size_lit = *size;
            let elem_size = inner.head_size();
            let elem_decodes: Vec<_> = (0..*size)
                .map(|i| {
                    let elem_offset = offset + i * elem_size;
                    generate_decode(inner, data_expr.clone(), elem_offset, use_alloc)
                })
                .collect();
            quote! {
                [#(#elem_decodes),*]
            }
        }
        SolType::Tuple(types) => {
            let mut current_offset = offset;
            let elem_decodes: Vec<_> = types
                .iter()
                .map(|t| {
                    let decode = generate_decode(t, data_expr.clone(), current_offset, use_alloc);
                    current_offset += t.head_size();
                    decode
                })
                .collect();
            quote! {
                (#(#elem_decodes),*)
            }
        }
    }
}

fn generate_decode_array_element(
    ty: &SolType,
    data_expr: TokenStream,
    use_alloc: bool,
) -> TokenStream {
    generate_decode(ty, data_expr, 0, use_alloc)
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
