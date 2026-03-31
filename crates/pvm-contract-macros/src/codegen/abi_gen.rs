use proc_macro2::TokenStream;
use quote::quote;

use super::contract::ParsedContract;
use super::dispatch::MethodInfo;
use crate::signature::SolType;

pub fn generate_abi_gen_main(parsed: &ParsedContract, has_sol_path: bool) -> TokenStream {
    if has_sol_path {
        return quote! {};
    }

    match generate_abi_gen_main_impl(parsed) {
        Ok(tokens) => tokens,
        Err(err) => err.to_compile_error(),
    }
}

fn generate_abi_gen_main_impl(parsed: &ParsedContract) -> syn::Result<TokenStream> {
    let has_custom_types = parsed
        .methods
        .iter()
        .flat_map(|method| {
            method
                .signature
                .inputs
                .iter()
                .chain(method.signature.outputs.iter())
        })
        .any(SolType::has_custom_types);

    let sol_encode_import = if has_custom_types {
        quote! {
            use ::pvm_contract_types::SolEncode;
        }
    } else {
        quote! {}
    };

    let constructor_entry = if parsed.has_constructor {
        let constructor_input_entries: Vec<TokenStream> = parsed
            .constructor_inputs
            .iter()
            .map(|(name, sol_type)| {
                let name_str = name.to_string();
                let type_name_expr = generate_sol_type_name_expr(sol_type)?;
                Ok(quote! {
                    if !__first_ctor_input {
                        __abi.push(',');
                    } else {
                        __first_ctor_input = false;
                    }
                    __abi.push_str("{\"name\":\"");
                    __abi.push_str(#name_str);
                    __abi.push_str("\",\"type\":\"");
                    __abi.push_str(&#type_name_expr);
                    __abi.push_str("\"}");
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;

        quote! {
            if !__first_item {
                __abi.push(',');
            } else {
                __first_item = false;
            }
            __abi.push_str("{\"type\":\"constructor\",\"inputs\":[");
            let mut __first_ctor_input = true;
            #(#constructor_input_entries)*
            __abi.push_str("]}");
        }
    } else {
        quote! {}
    };

    let method_entries: Vec<TokenStream> = parsed
        .methods
        .iter()
        .map(generate_method_entry)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        #[cfg(feature = "abi-gen")]
        fn main() {
            #sol_encode_import

            let mut __abi = ::std::string::String::from("[");
            let mut __first_item = true;

            #constructor_entry

            #(#method_entries)*

            __abi.push(']');
            ::std::println!("{}", __abi);
        }
    })
}

fn generate_method_entry(method: &MethodInfo) -> syn::Result<TokenStream> {
    let method_name = method.signature.name.clone();

    let input_entries: Vec<TokenStream> = method
        .signature
        .inputs
        .iter()
        .enumerate()
        .map(|(index, sol_type)| generate_input_entry(method, index, sol_type))
        .collect::<syn::Result<Vec<_>>>()?;

    let output_entries: Vec<TokenStream> = method
        .signature
        .outputs
        .iter()
        .map(generate_output_entry)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        if !__first_item {
            __abi.push(',');
        } else {
            __first_item = false;
        }

        __abi.push_str("{\"type\":\"function\",\"name\":\"");
        __abi.push_str(#method_name);
        __abi.push_str("\",\"inputs\":[");

        let mut __first_input = true;
        #(#input_entries)*

        __abi.push_str("],\"outputs\":[");

        let mut __first_output = true;
        #(#output_entries)*

        __abi.push_str("]}");
    })
}

fn generate_input_entry(
    method: &MethodInfo,
    index: usize,
    sol_type: &SolType,
) -> syn::Result<TokenStream> {
    let param_name = method
        .param_names
        .get(index)
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &method.fn_name,
                format!("Missing parameter name for input index {index}"),
            )
        })?
        .to_string();

    let type_name_expr = generate_sol_type_name_expr(sol_type)?;

    Ok(quote! {
        if !__first_input {
            __abi.push(',');
        } else {
            __first_input = false;
        }

        __abi.push_str("{\"name\":\"");
        __abi.push_str(#param_name);
        __abi.push_str("\",\"type\":\"");
        __abi.push_str(&#type_name_expr);
        __abi.push_str("\"}");
    })
}

fn generate_output_entry(sol_type: &SolType) -> syn::Result<TokenStream> {
    let type_name_expr = generate_sol_type_name_expr(sol_type)?;

    Ok(quote! {
        if !__first_output {
            __abi.push(',');
        } else {
            __first_output = false;
        }

        __abi.push_str("{\"name\":\"\",\"type\":\"");
        __abi.push_str(&#type_name_expr);
        __abi.push_str("\"}");
    })
}

fn generate_sol_type_name_expr(sol_type: &SolType) -> syn::Result<TokenStream> {
    if !sol_type.has_custom_types() {
        let canonical_name = sol_type.canonical_name();
        return Ok(quote! {
            ::std::string::String::from(#canonical_name)
        });
    }

    match sol_type {
        SolType::Custom(name) => {
            let ty = syn::parse_str::<syn::Type>(name).map_err(|error| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("Failed to parse custom SolType `{name}`: {error}"),
                )
            })?;

            Ok(quote! {
                <#ty as ::pvm_contract_types::SolEncode>::sol_name()
            })
        }
        SolType::Array(inner) => {
            let inner_expr = generate_sol_type_name_expr(inner)?;
            Ok(quote! {{
                let mut __type_name = #inner_expr;
                __type_name.push_str("[]");
                __type_name
            }})
        }
        SolType::FixedArray(inner, size) => {
            let inner_expr = generate_sol_type_name_expr(inner)?;
            let size = size.to_string();
            Ok(quote! {{
                let mut __type_name = #inner_expr;
                __type_name.push('[');
                __type_name.push_str(#size);
                __type_name.push(']');
                __type_name
            }})
        }
        SolType::Tuple(types) => {
            let inner_exprs: Vec<TokenStream> = types
                .iter()
                .map(generate_sol_type_name_expr)
                .collect::<syn::Result<Vec<_>>>()?;

            Ok(quote! {{
                let mut __type_name = ::std::string::String::from("(");
                let mut __first_tuple_item = true;
                #(
                    if !__first_tuple_item {
                        __type_name.push(',');
                    } else {
                        __first_tuple_item = false;
                    }
                    __type_name.push_str(&(#inner_exprs));
                )*
                __type_name.push(')');
                __type_name
            }})
        }
        _ => {
            let canonical_name = sol_type.canonical_name();
            Ok(quote! {
                ::std::string::String::from(#canonical_name)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_empty_stream_for_sol_path_contract() {
        let parsed = ParsedContract {
            mod_name: syn::parse_str("contract").unwrap(),
            methods: vec![],
            has_constructor: false,
            has_fallback: false,
            constructor_name: None,
            constructor_returns_result: false,
            constructor_inputs: vec![],
            fallback_name: None,
        };

        assert!(generate_abi_gen_main(&parsed, true).is_empty());
    }
}
