use proc_macro2::TokenStream;
use quote::quote;

use crate::signature::SolType;

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
                    // Solidity string encoding: offset (32) + length (32) + data (padded to 32)
                    let s: &str = #value_expr.as_str();
                    let len = s.len();
                    let padded_len = (len + 31) / 32 * 32;
                    let mut out = alloc::vec::Vec::with_capacity(64 + padded_len);
                    
                    // Encode offset
                    out.extend_from_slice(&[0u8; 31]);
                    out.push(32);
                    let mut len_bytes = [0u8; 32];
                    len_bytes[24..32].copy_from_slice(&(len as u64).to_be_bytes());
                    // Encode length
                    out.extend_from_slice(&len_bytes);
                    // Encode data + padding
                    out.extend_from_slice(s.as_bytes());
                    out.resize(64 + padded_len, 0);
                    out
                }}
            } else {
                panic!("String encoding requires alloc");
            }
        }
        SolType::DynBytes | SolType::Array(_) => {
            panic!("`DynBytes` & `Array` types require special handling in tuple encoding");
        }
        SolType::FixedArray(inner, size) => {
            let size_lit = *size;
            let inner_encodes: Vec<_> = (0..*size)
                .map(|i| {
                    let idx = i;
                    generate_encode(inner, quote!(#value_expr[#idx]), use_alloc)
                })
                .collect();
            if use_alloc {
                quote! {{
                    let mut out = alloc::vec::Vec::with_capacity(#size_lit * 32);
                    #(out.extend_from_slice(&#inner_encodes);)*
                    out
                }}
            } else {
                quote! {{
                    let mut out = [0u8; #size_lit * 32];
                    let mut offset = 0;
                    #(
                        out[offset..offset + 32].copy_from_slice(&#inner_encodes);
                        offset += 32;
                    )*
                    out
                }}
            }
        }
        SolType::Tuple(types) => {
            if types.iter().all(|t| !t.is_dynamic()) {
                let encodes: Vec<_> = types
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let idx = syn::Index::from(i);
                        generate_encode(t, quote!(#value_expr.#idx), use_alloc)
                    })
                    .collect();
                let total_size = types.iter().map(|t| t.head_size()).sum::<usize>();
                if use_alloc {
                    quote! {{
                        let mut out = alloc::vec::Vec::with_capacity(#total_size);
                        #(out.extend_from_slice(&#encodes);)*
                        out
                    }}
                } else {
                    quote! {{
                        let mut out = [0u8; #total_size];
                        let mut offset = 0;
                        #(
                            let encoded = #encodes;
                            out[offset..offset + 32].copy_from_slice(&encoded);
                            offset += 32;
                        )*
                        out
                    }}
                }
            } else {
                panic!("Dynamic tuple encoding not yet implemented");
            }
        }
    }
}

/// Encode a sequence of named parameters into ABI calldata.
/// Generates code that extends a pre-existing `calldata: Vec<u8>` variable.
/// Handles both all-static sequences (simple concatenation) and mixed
/// static/dynamic sequences (proper head/tail encoding).
pub fn generate_encode_params(
    names: &[syn::Ident],
    types: &[SolType],
) -> TokenStream {
    let has_dynamic = types.iter().any(|t| t.is_dynamic());

    if !has_dynamic {
        let encodes: Vec<TokenStream> = names
            .iter()
            .zip(types.iter())
            .map(|(name, ty)| {
                let enc = generate_encode(ty, quote!(#name), true);
                quote! { calldata.extend_from_slice(&#enc); }
            })
            .collect();
        quote! { #(#encodes)* }
    } else {
        let n_params = types.len();
        let head_size = n_params * 32;

        let mut writes = Vec::new();
        for (i, (name, ty)) in names.iter().zip(types.iter()).enumerate() {
            let offset = i * 32;
            let end = offset + 32;

            if ty.is_dynamic() {
                match ty {
                    SolType::String => {
                        writes.push(quote! {
                            {
                                let __dyn_offset = (#head_size + __tail.len()) as u64;
                                let mut __off = [0u8; 32];
                                __off[24..32].copy_from_slice(&__dyn_offset.to_be_bytes());
                                __head[#offset..#end].copy_from_slice(&__off);

                                let __s: &str = #name.as_str();
                                let __len = __s.len();
                                let __padded_len = (__len + 31) / 32 * 32;
                                let mut __len_bytes = [0u8; 32];
                                __len_bytes[24..32].copy_from_slice(&(__len as u64).to_be_bytes());
                                __tail.extend_from_slice(&__len_bytes);
                                __tail.extend_from_slice(__s.as_bytes());
                                __tail.resize(__tail.len() + __padded_len - __len, 0);
                            }
                        });
                    }
                    SolType::DynBytes => {
                        writes.push(quote! {
                            {
                                let __dyn_offset = (#head_size + __tail.len()) as u64;
                                let mut __off = [0u8; 32];
                                __off[24..32].copy_from_slice(&__dyn_offset.to_be_bytes());
                                __head[#offset..#end].copy_from_slice(&__off);

                                let __data: &[u8] = #name.as_ref();
                                let __len = __data.len();
                                let __padded_len = (__len + 31) / 32 * 32;
                                let mut __len_bytes = [0u8; 32];
                                __len_bytes[24..32].copy_from_slice(&(__len as u64).to_be_bytes());
                                __tail.extend_from_slice(&__len_bytes);
                                __tail.extend_from_slice(__data);
                                __tail.resize(__tail.len() + __padded_len - __len, 0);
                            }
                        });
                    }
                    SolType::Array(_) => {
                        // Dynamic array (`T[]`). Delegate to the runtime
                        // `SolAbi` impl on `Vec<T>` (crates/pvm_contract/src/abi.rs),
                        // which already handles both static-element arrays
                        // (length-prefixed + concatenated) and dynamic-element
                        // arrays (length-prefixed + head-of-pointers + tails).
                        writes.push(quote! {
                            {
                                let __dyn_offset = (#head_size + __tail.len()) as u64;
                                let mut __off = [0u8; 32];
                                __off[24..32].copy_from_slice(&__dyn_offset.to_be_bytes());
                                __head[#offset..#end].copy_from_slice(&__off);
                                pvm_contract::SolAbi::abi_encode(&#name, &mut __tail);
                            }
                        });
                    }
                    _ => panic!("Unsupported dynamic type in cross-contract call encoding"),
                }
            } else {
                let enc = generate_encode(ty, quote!(#name), true);
                writes.push(quote! {
                    __head[#offset..#end].copy_from_slice(&#enc);
                });
            }
        }

        quote! {
            {
                let mut __head = alloc::vec![0u8; #head_size];
                let mut __tail = alloc::vec::Vec::new();
                #(#writes)*
                calldata.extend_from_slice(&__head);
                calldata.extend_from_slice(&__tail);
            }
        }
    }
}

/// Encode parameters using the SolAbi trait (runtime dynamic detection).
/// Used when types aren't fully resolved at macro expansion time.
/// Generates code that extends a pre-existing `calldata: Vec<u8>` variable.
pub fn generate_encode_params_trait(
    names: &[syn::Ident],
    types: &[syn::Type],
) -> TokenStream {
    let n_params = types.len();
    let head_size = n_params * 32;

    let mut writes = Vec::new();
    for (i, (name, ty)) in names.iter().zip(types.iter()).enumerate() {
        let offset = i * 32;
        let end = offset + 32;
        writes.push(quote! {
            if <#ty as pvm_contract::SolAbi>::IS_DYNAMIC {
                let __dyn_offset = (#head_size + __tail.len()) as u64;
                let mut __off = [0u8; 32];
                __off[24..32].copy_from_slice(&__dyn_offset.to_be_bytes());
                __head[#offset..#end].copy_from_slice(&__off);
                <#ty as pvm_contract::SolAbi>::abi_encode(&#name, &mut __tail);
            } else {
                let mut __enc = alloc::vec::Vec::new();
                <#ty as pvm_contract::SolAbi>::abi_encode(&#name, &mut __enc);
                __head[#offset..#end].copy_from_slice(&__enc);
            }
        });
    }
    quote! {
        {
            let mut __head = alloc::vec![0u8; #head_size];
            let mut __tail = alloc::vec::Vec::new();
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

    if types.len() == 1 {
        let encode = generate_encode(&types[0], quote!(result), use_alloc);
        if use_alloc {
            return quote! { #encode.to_vec() };
        } else {
            return quote! { &#encode };
        }
    }

    let encodes: Vec<_> = types
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let idx = syn::Index::from(i);
            generate_encode(ty, quote!(result.#idx), use_alloc)
        })
        .collect();

    let total_size: usize = types.iter().map(|t| t.head_size()).sum();

    if use_alloc {
        quote! {{
            let mut out = alloc::vec::Vec::with_capacity(#total_size);
            #(out.extend_from_slice(&#encodes);)*
            out
        }}
    } else {
        quote! {{
            let mut out = [0u8; #total_size];
            let mut offset = 0;
            #(
                let encoded = #encodes;
                out[offset..offset + 32].copy_from_slice(&encoded);
                offset += 32;
            )*
            &out
        }}
    }
}
