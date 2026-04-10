use proc_macro2::TokenStream;
use quote::quote;

use super::sol_type::sol_type_head_size_expr;
use crate::signature::SolType;

fn generate_sol_decode<T: quote::ToTokens>(
    rust_type: T,
    data_expr: &TokenStream,
    offset: usize,
) -> TokenStream {
    let offset_lit = offset;
    quote! {
    <#rust_type as ::pvm_contract_types::SolDecode>::decode_at(&#data_expr, #offset_lit)
    }
}

fn generate_sol_decode_runtime<T: quote::ToTokens>(rust_type: T) -> TokenStream {
    quote! {
    <#rust_type as ::pvm_contract_types::SolDecode>::decode_at(&input, __decode_offset)
    }
}

pub fn generate_decode(
    ty: &SolType,
    data_expr: TokenStream,
    offset: usize,
    use_alloc: bool,
) -> TokenStream {
    let offset_lit = offset;

    match ty {
        SolType::Address => {
            generate_sol_decode(quote!(::pvm_contract_types::Address), &data_expr, offset)
        }
        SolType::Bool => generate_sol_decode(quote!(bool), &data_expr, offset),
        SolType::Uint(8) => generate_sol_decode(quote!(u8), &data_expr, offset),
        SolType::Uint(16) => generate_sol_decode(quote!(u16), &data_expr, offset),
        SolType::Uint(32) => generate_sol_decode(quote!(u32), &data_expr, offset),
        SolType::Uint(64) => generate_sol_decode(quote!(u64), &data_expr, offset),
        SolType::Uint(128) => generate_sol_decode(quote!(u128), &data_expr, offset),
        SolType::Uint(_) => generate_sol_decode(quote!(ruint::aliases::U256), &data_expr, offset),
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
                ::pvm_contract_types::I256::from_be_slice(&#data_expr[#offset_lit..#offset_lit + 32])
            }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            generate_sol_decode(quote!([u8; #size_lit]), &data_expr, offset)
        }
        SolType::DynBytes => {
            if use_alloc {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(#data_expr[#offset_lit + 24..#offset_lit + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(#data_expr[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    #data_expr[dyn_offset + 32..dyn_offset + 32 + length].to_vec()
                }}
            } else {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(#data_expr[#offset_lit + 24..#offset_lit + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(#data_expr[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    &#data_expr[dyn_offset + 32..dyn_offset + 32 + length]
                }}
            }
        }
        SolType::String => {
            if use_alloc {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(#data_expr[#offset_lit + 24..#offset_lit + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(#data_expr[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    let bytes = &#data_expr[dyn_offset + 32..dyn_offset + 32 + length];
                    alloc::string::String::from_utf8_lossy(bytes).into_owned()
                }}
            } else {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(#data_expr[#offset_lit + 24..#offset_lit + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(#data_expr[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    let bytes = &#data_expr[dyn_offset + 32..dyn_offset + 32 + length];
                    core::str::from_utf8(bytes).unwrap_or("")
                }}
            }
        }
        SolType::Array(inner) => {
            if use_alloc {
                let inner_decode =
                    generate_decode_array_element(inner, quote!(elem_data), use_alloc);
                let elem_size = sol_type_head_size_expr(inner);
                quote! {{
                    let dyn_offset = u64::from_be_bytes(#data_expr[#offset_lit + 24..#offset_lit + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(#data_expr[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    let array_data = &#data_expr[dyn_offset + 32..];
                    let __elem_size: usize = #elem_size;
                    let mut result = alloc::vec::Vec::with_capacity(length);
                    for i in 0..length {
                        let elem_data = &array_data[i * __elem_size..];
                        result.push(#inner_decode);
                    }
                    result
                }}
            } else {
                panic!("Dynamic arrays require an explicit allocator");
            }
        }
        SolType::FixedArray(inner, size) => {
            if inner.has_custom_types() {
                let elem_size_expr = sol_type_head_size_expr(inner);
                let indices: Vec<_> = (0..*size).collect();
                let inner_decode =
                    generate_decode_array_element(inner, quote!(__elem_data), use_alloc);
                quote! {{
                    let __es: usize = #elem_size_expr;
                    [#({
                        let __elem_data = &#data_expr[#offset_lit + #indices * __es..];
                        #inner_decode
                    }),*]
                }}
            } else {
                let elem_size = inner.head_size().expect("FixedArray inner head_size");
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
        }
        SolType::Tuple(types) => {
            let has_custom = types.iter().any(|t| t.has_custom_types());
            if has_custom {
                let elem_decodes: Vec<TokenStream> = types
                    .iter()
                    .map(|t| {
                        let decode = generate_decode_runtime_offset(t, use_alloc);
                        let size_increment = sol_type_head_size_expr(t);
                        quote! {{
                            let __value = #decode;
                            __decode_offset += #size_increment;
                            __value
                        }}
                    })
                    .collect();
                quote! {{
                    let __base = #offset_lit;
                    let mut __decode_offset: usize = __base;
                    let input = &#data_expr;
                    (#(#elem_decodes),*)
                }}
            } else {
                let mut current_offset = offset;
                let elem_decodes: Vec<_> = types
                    .iter()
                    .map(|t| {
                        let decode =
                            generate_decode(t, data_expr.clone(), current_offset, use_alloc);
                        current_offset += t.head_size().expect("Tuple inner head_size");
                        decode
                    })
                    .collect();
                quote! {
                    (#(#elem_decodes),*)
                }
            }
        }
        SolType::Custom(name) => match syn::parse_str::<syn::Path>(name) {
            Ok(type_path) => {
                quote! {
                <#type_path as ::pvm_contract_types::SolDecode>::decode_at(&#data_expr, #offset_lit)
                }
            }
            Err(err) => {
                let msg = format!("Invalid custom type path `{name}` in decode codegen: {err}");
                quote! {{
                    compile_error!(#msg);
                    ::core::unreachable!()
                }}
            }
        },
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
    let has_custom = types.iter().any(|t| t.has_custom_types());

    if has_custom {
        generate_decode_params_with_runtime_offset(types, use_alloc)
    } else {
        let mut offset = 0;
        types
            .iter()
            .map(|ty| {
                let decode = generate_decode(ty, quote!(input), offset, use_alloc);
                offset += ty
                    .head_size()
                    .expect("Param decode called on unresolved custom type");
                decode
            })
            .collect()
    }
}

fn generate_decode_params_with_runtime_offset(
    types: &[SolType],
    use_alloc: bool,
) -> Vec<TokenStream> {
    types
        .iter()
        .map(|ty| {
            let decode = generate_decode_runtime_offset(ty, use_alloc);
            let size_increment = sol_type_head_size_expr(ty);
            quote! {{
                let __value = #decode;
                __decode_offset += #size_increment;
                __value
            }}
        })
        .collect()
}

fn generate_decode_runtime_offset(ty: &SolType, use_alloc: bool) -> TokenStream {
    match ty {
        SolType::Address => generate_sol_decode_runtime(quote!(::pvm_contract_types::Address)),
        SolType::Bool => generate_sol_decode_runtime(quote!(bool)),
        SolType::Uint(8) => generate_sol_decode_runtime(quote!(u8)),
        SolType::Uint(16) => generate_sol_decode_runtime(quote!(u16)),
        SolType::Uint(32) => generate_sol_decode_runtime(quote!(u32)),
        SolType::Uint(64) => generate_sol_decode_runtime(quote!(u64)),
        SolType::Uint(128) => generate_sol_decode_runtime(quote!(u128)),
        SolType::Uint(_) => generate_sol_decode_runtime(quote!(ruint::aliases::U256)),
        SolType::Int(8) => quote! { input[__decode_offset + 31] as i8 },
        SolType::Int(16) => {
            quote! { i16::from_be_bytes([input[__decode_offset + 30], input[__decode_offset + 31]]) }
        }
        SolType::Int(32) => {
            quote! { i32::from_be_bytes(input[__decode_offset + 28..__decode_offset + 32].try_into().unwrap()) }
        }
        SolType::Int(64) => {
            quote! { i64::from_be_bytes(input[__decode_offset + 24..__decode_offset + 32].try_into().unwrap()) }
        }
        SolType::Int(128) => {
            quote! { i128::from_be_bytes(input[__decode_offset + 16..__decode_offset + 32].try_into().unwrap()) }
        }
        SolType::Int(_) => {
            quote! { ::pvm_contract_types::I256::from_be_slice(&input[__decode_offset..__decode_offset + 32]) }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            generate_sol_decode_runtime(quote!([u8; #size_lit]))
        }
        SolType::DynBytes => {
            if use_alloc {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(input[__decode_offset + 24..__decode_offset + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(input[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    input[dyn_offset + 32..dyn_offset + 32 + length].to_vec()
                }}
            } else {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(input[__decode_offset + 24..__decode_offset + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(input[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    &input[dyn_offset + 32..dyn_offset + 32 + length]
                }}
            }
        }
        SolType::String => {
            if use_alloc {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(input[__decode_offset + 24..__decode_offset + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(input[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    let bytes = &input[dyn_offset + 32..dyn_offset + 32 + length];
                    alloc::string::String::from_utf8_lossy(bytes).into_owned()
                }}
            } else {
                quote! {{
                    let dyn_offset = u64::from_be_bytes(input[__decode_offset + 24..__decode_offset + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(input[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    let bytes = &input[dyn_offset + 32..dyn_offset + 32 + length];
                    core::str::from_utf8(bytes).unwrap_or("")
                }}
            }
        }
        SolType::Array(inner) => {
            if use_alloc {
                let inner_decode =
                    generate_decode_array_element(inner, quote!(elem_data), use_alloc);
                let elem_size = sol_type_head_size_expr(inner);
                quote! {{
                    let dyn_offset = u64::from_be_bytes(input[__decode_offset + 24..__decode_offset + 32].try_into().unwrap()) as usize;
                    let length = u64::from_be_bytes(input[dyn_offset + 24..dyn_offset + 32].try_into().unwrap()) as usize;
                    let array_data = &input[dyn_offset + 32..];
                    let __elem_size: usize = #elem_size;
                    let mut result = alloc::vec::Vec::with_capacity(length);
                    for i in 0..length {
                        let elem_data = &array_data[i * __elem_size..];
                        result.push(#inner_decode);
                    }
                    result
                }}
            } else {
                panic!("Dynamic arrays require an explicit allocator");
            }
        }
        SolType::Custom(name) => match syn::parse_str::<syn::Path>(name) {
            Ok(type_path) => {
                quote! { <#type_path as ::pvm_contract_types::SolDecode>::decode_at(&input, __decode_offset) }
            }
            Err(err) => {
                let msg = format!("Invalid custom type path `{name}` in decode codegen: {err}");
                quote! {{
                    compile_error!(#msg);
                    ::core::unreachable!()
                }}
            }
        },
        SolType::FixedArray(inner, size) => {
            let elem_size_expr = sol_type_head_size_expr(inner);
            let indices: Vec<_> = (0..*size).collect();
            let inner_decode =
                generate_decode_array_element(inner, quote!(__fa_elem_data), use_alloc);
            quote! {{
                let __fa_es: usize = #elem_size_expr;
                [#({
                    let __fa_elem_data = &input[__decode_offset + #indices * __fa_es..];
                    #inner_decode
                }),*]
            }}
        }
        SolType::Tuple(types) => {
            let elem_decodes: Vec<TokenStream> = types
                .iter()
                .map(|t| {
                    let decode = generate_decode_runtime_offset(t, use_alloc);
                    let size_increment = sol_type_head_size_expr(t);
                    quote! {{
                        let __value = #decode;
                        __decode_offset += #size_increment;
                        __value
                    }}
                })
                .collect();
            quote! {{
                (#(#elem_decodes),*)
            }}
        }
    }
}

pub fn calculate_min_input_size(types: &[SolType]) -> TokenStream {
    let has_custom = types.iter().any(|t| t.has_custom_types());

    if !has_custom {
        let total: usize = types
            .iter()
            .map(|t| {
                t.head_size()
                    .expect("calculate_min_input_size called on unresolved custom type")
            })
            .sum();
        return quote! { #total };
    }

    let size_exprs: Vec<TokenStream> = types.iter().map(sol_type_head_size_expr).collect();

    quote! { 0 #(+ #size_exprs)* }
}

pub fn has_custom_types(types: &[SolType]) -> bool {
    types.iter().any(|t| t.has_custom_types())
}
