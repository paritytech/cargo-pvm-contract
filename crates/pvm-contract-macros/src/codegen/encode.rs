use proc_macro2::TokenStream;
use quote::quote;

use super::sol_type::{sol_type_head_size_expr, sol_type_is_dynamic_expr};
use crate::signature::SolType;

fn generate_sol_encode<T: quote::ToTokens>(rust_type: T, value_expr: &TokenStream) -> TokenStream {
    quote! {{
        let mut __buf = [0u8; <#rust_type as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
        <#rust_type as ::pvm_contract_types::SolEncode>::encode_to(&#value_expr, &mut __buf);
        __buf
    }}
}

pub fn generate_encode(ty: &SolType, value_expr: TokenStream, use_alloc: bool) -> TokenStream {
    match ty {
        SolType::Address => generate_sol_encode(quote!(::pvm_contract_types::Address), &value_expr),
        SolType::Bool => generate_sol_encode(quote!(bool), &value_expr),
        SolType::Uint(8) => generate_sol_encode(quote!(u8), &value_expr),
        SolType::Uint(16) => generate_sol_encode(quote!(u16), &value_expr),
        SolType::Uint(32) => generate_sol_encode(quote!(u32), &value_expr),
        SolType::Uint(64) => generate_sol_encode(quote!(u64), &value_expr),
        SolType::Uint(128) => generate_sol_encode(quote!(u128), &value_expr),
        SolType::Uint(_) => generate_sol_encode(quote!(ruint::aliases::U256), &value_expr),
        SolType::Bytes(size) => {
            let s = *size;
            generate_sol_encode(quote!([u8; #s]), &value_expr)
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
        SolType::Int(bits) if *bits <= 128 => {
            let start = 32 - bits / 8;
            quote! {{
                let mut out = if #value_expr < 0 { [0xff; 32] } else { [0u8; 32] };
                out[#start..32].copy_from_slice(&#value_expr.to_be_bytes());
                out
            }}
        }
        SolType::Int(_) => {
            quote! {
                #value_expr.to_be_bytes::<32>()
            }
        }
        SolType::DynBytes | SolType::String | SolType::Array(_) => {
            panic!("Dynamic types require special handling in tuple encoding");
        }
        SolType::Custom(name) => match syn::parse_str::<syn::Path>(name) {
            Ok(type_path) => {
                if use_alloc {
                    quote! {{
                        let __len = ::pvm_contract_types::SolEncode::encode_len(&#value_expr);
                        let mut __buf = alloc::vec![0u8; __len];
                        ::pvm_contract_types::SolEncode::encode_to(&#value_expr, &mut __buf);
                        __buf
                    }}
                } else {
                    quote! {{
                        let mut __buf = [0u8; <#type_path as ::pvm_contract_types::StaticEncodedLen>::ENCODED_SIZE];
                        <#type_path as ::pvm_contract_types::SolEncode>::encode_to(&#value_expr, &mut __buf);
                        __buf
                    }}
                }
            }
            Err(err) => {
                let msg = format!("Invalid custom type path `{name}` in encode codegen: {err}");
                quote! {{
                    compile_error!(#msg);
                    ::core::unreachable!()
                }}
            }
        },
        SolType::FixedArray(inner, size) => {
            let size_lit = *size;
            let elem_size = sol_type_head_size_expr(inner);
            let inner_encodes: Vec<_> = (0..*size)
                .map(|i| {
                    let idx = i;
                    generate_encode(inner, quote!(#value_expr[#idx]), use_alloc)
                })
                .collect();
            if use_alloc {
                quote! {{
                    let mut out = alloc::vec::Vec::with_capacity(#size_lit * #elem_size);
                    #(out.extend_from_slice(&#inner_encodes);)*
                    out
                }}
            } else {
                quote! {{
                    let mut out = [0u8; #size_lit * #elem_size];
                    let mut offset = 0;
                    #(
                        let encoded = #inner_encodes;
                        out[offset..offset + encoded.len()].copy_from_slice(&encoded);
                        offset += encoded.len();
                    )*
                    out
                }}
            }
        }
        SolType::Tuple(types) => {
            if types.iter().all(|t| t.is_dynamic() == Some(false)) {
                let encodes: Vec<_> = types
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let idx = syn::Index::from(i);
                        generate_encode(t, quote!(#value_expr.#idx), use_alloc)
                    })
                    .collect();
                let total_size = types
                    .iter()
                    .map(|t| {
                        t.head_size()
                            .expect("Tuple static encode called on unresolved custom type")
                    })
                    .sum::<usize>();
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
                            out[offset..offset + encoded.len()].copy_from_slice(&encoded);
                            offset += encoded.len();
                        )*
                        out
                    }}
                }
            } else {
                if !use_alloc {
                    return quote! {
                        compile_error!("Tuple contains dynamic or custom types and requires an explicit allocator. Set `allocator = \"pico\"` or `allocator = \"bump\"` in `#[contract]`.")
                    };
                }
                let head_size_parts: Vec<TokenStream> = types
                    .iter()
                    .map(|t| {
                        if t.is_dynamic() == Some(true) {
                            quote! { 32usize }
                        } else {
                            sol_type_head_size_expr(t)
                        }
                    })
                    .collect();
                let head_size_expr = quote! { (0 #(+ #head_size_parts)*) };
                let mut stmts = Vec::new();

                for (i, t) in types.iter().enumerate() {
                    let idx = syn::Index::from(i);
                    let elem_expr = quote!(#value_expr.#idx);

                    if t.is_dynamic() == Some(true) {
                        stmts.push(quote! {{
                            let __off = __head_size + __tail.len();
                            let mut __off_buf = [0u8; 32];
                            __off_buf[24..32].copy_from_slice(&(__off as u64).to_be_bytes());
                            __head.extend_from_slice(&__off_buf);
                            let __tl = ::pvm_contract_types::SolEncode::tail_len(&#elem_expr);
                            let mut __tbuf = alloc::vec![0u8; __tl];
                            ::pvm_contract_types::SolEncode::encode_tail_to(&#elem_expr, &mut __tbuf);
                            __tail.extend_from_slice(&__tbuf);
                        }});
                    } else if t.is_dynamic() == Some(false) {
                        let encode = generate_encode(t, elem_expr, use_alloc);
                        stmts.push(quote! {
                            __head.extend_from_slice(&#encode);
                        });
                    } else {
                        let is_dyn_expr = sol_type_is_dynamic_expr(t);
                        let hs_expr = sol_type_head_size_expr(t);
                        stmts.push(quote! {
                            if #is_dyn_expr {
                                let __off = __head_size + __tail.len();
                                let mut __off_buf = [0u8; 32];
                                __off_buf[24..32].copy_from_slice(&(__off as u64).to_be_bytes());
                                __head.extend_from_slice(&__off_buf);
                                let __tl = ::pvm_contract_types::SolEncode::tail_len(&#elem_expr);
                                let mut __tbuf = alloc::vec![0u8; __tl];
                                ::pvm_contract_types::SolEncode::encode_tail_to(&#elem_expr, &mut __tbuf);
                                __tail.extend_from_slice(&__tbuf);
                            } else {
                                let __hs = #hs_expr;
                                let __start = __head.len();
                                __head.resize(__start + __hs, 0);
                                ::pvm_contract_types::SolEncode::encode_to(
                                    &#elem_expr,
                                    &mut __head[__start..__start + __hs],
                                );
                            }
                        });
                    }
                }

                quote! {{
                    let __head_size: usize = #head_size_expr;
                    let mut __head = alloc::vec::Vec::with_capacity(__head_size);
                    let mut __tail = alloc::vec::Vec::new();
                    #(#stmts)*
                    __head.extend_from_slice(&__tail);
                    __head
                }}
            }
        }
    }
}

pub fn generate_dynamic_value_encode(ty: &SolType, value_expr: TokenStream) -> TokenStream {
    match ty {
        SolType::String => {
            quote! {{
                let bytes = #value_expr.as_bytes();
                let mut out = alloc::vec::Vec::new();
                let len = bytes.len();
                let len_value = ruint::aliases::U256::from(len);
                let mut len_buf = [0u8; 32];
                <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(&len_value, &mut len_buf);
                out.extend_from_slice(&len_buf);
                out.extend_from_slice(bytes);
                let padding = (32 - (len % 32)) % 32;
                out.extend(core::iter::repeat(0u8).take(padding));
                out
            }}
        }
        SolType::DynBytes => {
            quote! {{
                let bytes: &[u8] = #value_expr.as_ref();
                let mut out = alloc::vec::Vec::new();
                let len = bytes.len();
                let len_value = ruint::aliases::U256::from(len);
                let mut len_buf = [0u8; 32];
                <ruint::aliases::U256 as ::pvm_contract_types::SolEncode>::encode_to(&len_value, &mut len_buf);
                out.extend_from_slice(&len_buf);
                out.extend_from_slice(bytes);
                let padding = (32 - (len % 32)) % 32;
                out.extend(core::iter::repeat(0u8).take(padding));
                out
            }}
        }
        SolType::Array(_) => {
            quote! {{
                let __arr = &#value_expr;
                let __tl = ::pvm_contract_types::SolEncode::tail_len(__arr);
                let mut out = alloc::vec![0u8; __tl];
                ::pvm_contract_types::SolEncode::encode_tail_to(__arr, &mut out);
                out
            }}
        }
        _ => {
            panic!("generate_dynamic_value_encode called with non-dynamic type");
        }
    }
}
