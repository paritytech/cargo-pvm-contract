use proc_macro2::TokenStream;
use quote::quote;

use crate::signature::SolType;

fn generate_encode_sequence(
    value_exprs: &[TokenStream],
    types: &[SolType],
    use_alloc: bool,
) -> TokenStream {
    let has_dynamic = types.iter().any(|t| t.is_dynamic());
    let head_size = types.iter().map(|t| t.head_size()).sum::<usize>();

    if !has_dynamic {
        let encodes: Vec<_> = value_exprs
            .iter()
            .zip(types.iter())
            .map(|(value_expr, ty)| generate_encode(ty, value_expr.clone(), use_alloc))
            .collect();
        let slot_sizes: Vec<_> = types.iter().map(|ty| ty.head_size()).collect();

        if use_alloc {
            quote! {{
                let mut out = alloc::vec::Vec::with_capacity(#head_size);
                #(out.extend_from_slice(&#encodes);)*
                out
            }}
        } else {
            quote! {{
                let mut out = [0u8; #head_size];
                let mut offset = 0usize;
                #(
                    let encoded = #encodes;
                    out[offset..offset + #slot_sizes].copy_from_slice(&encoded);
                    offset += #slot_sizes;
                )*
                out
            }}
        }
    } else {
        if !use_alloc {
            panic!("Dynamic ABI encoding requires alloc");
        }

        let mut writes = Vec::new();
        let mut offset = 0usize;
        for (value_expr, ty) in value_exprs.iter().zip(types.iter()) {
            let slot_size = ty.head_size();
            let end = offset + slot_size;
            let encoded = generate_encode(ty, value_expr.clone(), true);

            if ty.is_dynamic() {
                writes.push(quote! {
                    {
                        let __dyn_offset = (#head_size + __tail.len()) as u64;
                        let mut __off = [0u8; 32];
                        __off[24..32].copy_from_slice(&__dyn_offset.to_be_bytes());
                        __head[#offset..#offset + 32].copy_from_slice(&__off);

                        let __enc = #encoded;
                        __tail.extend_from_slice(&__enc);
                    }
                });
            } else {
                writes.push(quote! {
                    {
                        let __enc = #encoded;
                        __head[#offset..#end].copy_from_slice(&__enc);
                    }
                });
            }

            offset = end;
        }

        quote! {{
            let mut __head = alloc::vec![0u8; #head_size];
            let mut __tail = alloc::vec::Vec::new();
            #(#writes)*
            let mut out = alloc::vec::Vec::with_capacity(#head_size + __tail.len());
            out.extend_from_slice(&__head);
            out.extend_from_slice(&__tail);
            out
        }}
    }
}

pub fn generate_encode(ty: &SolType, value_expr: TokenStream, use_alloc: bool) -> TokenStream {
    match ty {
        SolType::Address => {
            quote! {{
                let mut out = [0u8; 32];
                out[12..32].copy_from_slice(#value_expr.as_bytes());
                out
            }}
        }
        SolType::Bool => {
            quote! {{
                let mut out = [0u8; 32];
                out[31] = if #value_expr { 1 } else { 0 };
                out
            }}
        }
        SolType::Uint(8) => {
            quote! {{
                let mut out = [0u8; 32];
                out[31] = #value_expr;
                out
            }}
        }
        SolType::Uint(16) => {
            quote! {{
                let mut out = [0u8; 32];
                out[30..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Uint(32) => {
            quote! {{
                let mut out = [0u8; 32];
                out[28..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Uint(64) => {
            quote! {{
                let mut out = [0u8; 32];
                out[24..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Uint(128) => {
            quote! {{
                let mut out = [0u8; 32];
                out[16..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Uint(_) => {
            quote! {
                #value_expr.to_be_bytes::<32>()
            }
        }
        SolType::Int(8) => {
            quote! {{
                let mut out = [0u8; 32];
                if #value_expr < 0 {
                    out = [0xff; 32];
                }
                out[31] = #value_expr as u8;
                out
            }}
        }
        SolType::Int(16) => {
            quote! {{
                let mut out = if #value_expr < 0 { [0xff; 32] } else { [0u8; 32] };
                out[30..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Int(32) => {
            quote! {{
                let mut out = if #value_expr < 0 { [0xff; 32] } else { [0u8; 32] };
                out[28..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Int(64) => {
            quote! {{
                let mut out = if #value_expr < 0 { [0xff; 32] } else { [0u8; 32] };
                out[24..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Int(128) => {
            quote! {{
                let mut out = if #value_expr < 0 { [0xff; 32] } else { [0u8; 32] };
                out[16..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Int(_) => {
            quote! {
                #value_expr.to_be_bytes::<32>()
            }
        }
        SolType::Bytes(size) => {
            let size_lit = *size;
            quote! {{
                let mut out = [0u8; 32];
                out[..#size_lit].copy_from_slice(&#value_expr);
                out
            }}
        }
        SolType::String => {
            if use_alloc {
                quote! {{
                    let __string: &str = #value_expr.as_str();
                    let __len = __string.len();
                    let __padded_len = (__len + 31) / 32 * 32;
                    let mut out = alloc::vec::Vec::with_capacity(32 + __padded_len);

                    let mut __len_bytes = [0u8; 32];
                    __len_bytes[24..32].copy_from_slice(&(__len as u64).to_be_bytes());
                    out.extend_from_slice(&__len_bytes);
                    out.extend_from_slice(__string.as_bytes());
                    out.resize(32 + __padded_len, 0);
                    out
                }}
            } else {
                panic!("String encoding requires alloc");
            }
        }
        SolType::DynBytes => {
            if use_alloc {
                quote! {{
                    let __bytes: &[u8] = &#value_expr;
                    let __len = __bytes.len();
                    let __padded_len = (__len + 31) / 32 * 32;
                    let mut out = alloc::vec::Vec::with_capacity(32 + __padded_len);

                    let mut __len_bytes = [0u8; 32];
                    __len_bytes[24..32].copy_from_slice(&(__len as u64).to_be_bytes());
                    out.extend_from_slice(&__len_bytes);
                    out.extend_from_slice(__bytes);
                    out.resize(32 + __padded_len, 0);
                    out
                }}
            } else {
                panic!("Dynamic bytes encoding requires alloc");
            }
        }
        SolType::Array(inner) => {
            if !use_alloc {
                panic!("Dynamic arrays require alloc");
            }

            let inner_slot_size = inner.head_size();
            if inner.is_dynamic() {
                let inner_encode = generate_encode(inner, quote!(__item), true);
                quote! {{
                    let __array = &#value_expr;
                    let __len = __array.len();
                    let __array_head_size = __len * #inner_slot_size;
                    let mut out = alloc::vec::Vec::new();

                    let mut __len_bytes = [0u8; 32];
                    __len_bytes[24..32].copy_from_slice(&(__len as u64).to_be_bytes());
                    out.extend_from_slice(&__len_bytes);

                    let mut __head = alloc::vec![0u8; __array_head_size];
                    let mut __tail = alloc::vec::Vec::new();
                    for (i, __item) in __array.iter().enumerate() {
                        let __offset = i * #inner_slot_size;
                        let __dyn_offset = (__array_head_size + __tail.len()) as u64;
                        let mut __off = [0u8; 32];
                        __off[24..32].copy_from_slice(&__dyn_offset.to_be_bytes());
                        __head[__offset..__offset + 32].copy_from_slice(&__off);

                        let __enc = #inner_encode;
                        __tail.extend_from_slice(&__enc);
                    }

                    out.extend_from_slice(&__head);
                    out.extend_from_slice(&__tail);
                    out
                }}
            } else {
                let inner_encode = generate_encode(inner, quote!(__item), true);
                quote! {{
                    let __array = &#value_expr;
                    let __len = __array.len();
                    let mut out = alloc::vec::Vec::new();

                    let mut __len_bytes = [0u8; 32];
                    __len_bytes[24..32].copy_from_slice(&(__len as u64).to_be_bytes());
                    out.extend_from_slice(&__len_bytes);

                    for __item in __array.iter() {
                        let __enc = #inner_encode;
                        out.extend_from_slice(&__enc);
                    }
                    out
                }}
            }
        }
        SolType::FixedArray(inner, size) => {
            let element_types = vec![(**inner).clone(); *size];
            let element_exprs: Vec<_> = (0..*size)
                .map(|i| {
                    let idx = syn::Index::from(i);
                    quote!(#value_expr[#idx])
                })
                .collect();
            generate_encode_sequence(&element_exprs, &element_types, use_alloc)
        }
        SolType::Tuple(types) => {
            let element_exprs: Vec<_> = types
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let idx = syn::Index::from(i);
                    quote!(#value_expr.#idx)
                })
                .collect();
            generate_encode_sequence(&element_exprs, types, use_alloc)
        }
    }
}

/// Encode a sequence of named parameters into ABI calldata.
/// Generates code that extends a pre-existing `calldata: Vec<u8>` variable.
pub fn generate_encode_params(names: &[syn::Ident], types: &[SolType]) -> TokenStream {
    let value_exprs: Vec<_> = names.iter().map(|name| quote!(#name)).collect();
    let encoded = generate_encode_sequence(&value_exprs, types, true);
    quote! {
        calldata.extend_from_slice(&#encoded);
    }
}

/// Encode parameters using the SolAbi trait (runtime dynamic detection).
/// Used when types aren't fully resolved at macro expansion time.
/// Generates code that extends a pre-existing `calldata: Vec<u8>` variable.
pub fn generate_encode_params_trait(names: &[syn::Ident], types: &[syn::Type]) -> TokenStream {
    let head_size_exprs: Vec<_> = types
        .iter()
        .map(|ty| quote! { <#ty as pvm_contract::SolAbi>::SLOT_SIZE })
        .collect();

    let writes: Vec<_> = names
        .iter()
        .zip(types.iter())
        .map(|(name, ty)| {
            quote! {
                {
                    let __slot_size: usize = <#ty as pvm_contract::SolAbi>::SLOT_SIZE;
                    if <#ty as pvm_contract::SolAbi>::IS_DYNAMIC {
                        let __dyn_offset = (__head_size + __tail.len()) as u64;
                        let mut __off = [0u8; 32];
                        __off[24..32].copy_from_slice(&__dyn_offset.to_be_bytes());
                        __head[__offset..__offset + 32].copy_from_slice(&__off);
                        <#ty as pvm_contract::SolAbi>::abi_encode(&#name, &mut __tail);
                    } else {
                        let mut __enc = alloc::vec::Vec::new();
                        <#ty as pvm_contract::SolAbi>::abi_encode(&#name, &mut __enc);
                        __head[__offset..__offset + __slot_size].copy_from_slice(&__enc);
                    }
                    __offset += __slot_size;
                }
            }
        })
        .collect();

    quote! {
        {
            let __head_size: usize = 0usize #(+ #head_size_exprs)*;
            let mut __head = alloc::vec![0u8; __head_size];
            let mut __tail = alloc::vec::Vec::new();
            let mut __offset = 0usize;
            #(#writes)*
            calldata.extend_from_slice(&__head);
            calldata.extend_from_slice(&__tail);
        }
    }
}

pub fn generate_encode_return(types: &[SolType], use_alloc: bool) -> TokenStream {
    if types.is_empty() {
        return quote! { &[] };
    }

    let value_exprs: Vec<_> = if types.len() == 1 {
        vec![quote!(result)]
    } else {
        types
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let idx = syn::Index::from(i);
                quote!(result.#idx)
            })
            .collect()
    };

    let encoded = generate_encode_sequence(&value_exprs, types, use_alloc);
    if use_alloc {
        quote! { #encoded }
    } else {
        quote! { &#encoded }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::format_ident;

    #[test]
    fn encode_return_uses_head_tail_for_mixed_outputs() {
        let tokens =
            generate_encode_return(&[SolType::Uint(64), SolType::String], true).to_string();
        assert!(tokens.contains("__head"));
        assert!(tokens.contains("__tail"));
    }

    #[test]
    fn encode_params_supports_tuple_and_dynamic_array_inputs() {
        let params = vec![format_ident!("player"), format_ident!("ghosts")];
        let types = vec![
            SolType::Tuple(vec![SolType::String, SolType::Uint(64)]),
            SolType::Array(Box::new(SolType::Tuple(vec![
                SolType::String,
                SolType::Uint(64),
            ]))),
        ];

        let tokens = generate_encode_params(&params, &types).to_string();
        assert!(tokens.contains("__array"));
        assert!(tokens.contains("__tail"));
    }
}
