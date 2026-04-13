use alloy_json_abi::{self, ToSolConfig};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json;
use std::path::Path;
use syn::{self};
use syn_solidity::{File, ItemFunction, SolIdent};

use crate::signature::compute_selector;
use crate::solidity::{capitalize, to_snake_case};

pub fn load_json_abi(
    name: String,
    token_span: proc_macro2::Span,
    path: &Path,
) -> Result<File, syn::Error> {
    let file = std::fs::read_to_string(path)
        .map_err(|err| syn::Error::new(token_span, err.to_string()))?;

    let parsed: alloy_json_abi::JsonAbi =
        serde_json::from_str(&file).map_err(|err| syn::Error::new(token_span, err.to_string()))?;
    let config = ToSolConfig::new()
        .print_constructors(true)
        .for_sol_macro(true);

    let unparsed = &parsed.to_sol(&name, Some(config));
    let tts = syn::parse_str::<TokenStream>(unparsed)
        .map_err(|e| syn::Error::new(token_span, &e.to_string()))?;

    syn_solidity::parse2(quote! {
        #tts
    })
}

pub fn expand_function(
    contract_name: syn::Ident,
    func: &ItemFunction,
    is_constructor: bool,
) -> TokenStream {
    let func_name = if is_constructor {
        format_ident!("{}", "new")
    } else {
        format_ident!("{}", to_snake_case(&func.name().to_string()))
    };
    let selector: Vec<TokenStream> = if is_constructor {
        [0u8; 4].into_iter().map(|x| quote! { #x }).collect()
    } else {
        compute_selector(&format!("{}{}", func.name(), func.call_type()))
            .into_iter()
            .map(|x| quote! { #x })
            .collect()
    };
    let args = if func.parameters.is_empty() {
        quote! {}
    } else {
        let args = func.parameters.iter().map(|param| {
            let typ = to_rust_type(&param.ty);
            let name = &param
                .name
                .as_ref()
                .unwrap_or(&SolIdent::new("s"))
                .to_string();
            let name = format_ident!("{}", name);
            quote! {, #name: #typ}
        });
        quote! { #(#args)* }
    };

    let return_type = if let Some(ret) = func.return_type() {
        let typ = to_rust_type(&ret);
        quote! { #typ}
    } else {
        quote! { () }
    };

    let self_ = quote! {mut self};

    let types = func.parameters.types().map(to_rust_type);
    let names = func.parameters.names().map(|name| {
        let name = name.as_ref().map_or(&SolIdent::new("s"), |v| v).to_string();
        format_ident!("{}", name)
    });

    let state_mutability = if let Some(mutability) = func.attributes.mutability() {
        match mutability {
            syn_solidity::Mutability::Pure(_) => quote! {
                Pure
            },
            syn_solidity::Mutability::View(_) => {
                quote! {
                    View
                }
            }
            syn_solidity::Mutability::Payable(_) => {
                quote! {
                    Payable
                }
            }
            syn_solidity::Mutability::Constant(_) => {
                quote! {
                    compile_error!("constant mutability no supported")
                }
            }
        }
    } else {
        quote! {
            NonPayable
        }
    };
    let t: Vec<TokenStream> = types.clone().collect();
    let res = quote! {
        pub fn #func_name(#self_ #args) -> #contract_name<#state_mutability, ( #(#types),* ), #return_type, true> {
            #contract_name::<#state_mutability, ( #(#t),* ), #return_type, true> {
                address: self.address,
                call_builder: CallBuilder::<#state_mutability, ( #(#t),* ), #return_type> {
                    payload: (#(#names),*),
                    selector: [#(#selector),*],
                    witness: #state_mutability::default(),
                    call_limits: Default::default(),
                    _ret: core::marker::PhantomData,
                }
            }
        }
    };
    res
}

fn to_rust_type(typ: &syn_solidity::Type) -> TokenStream {
    match typ {
        syn_solidity::Type::Address(_span, _payable) => quote! { Address },
        syn_solidity::Type::Bool(_) => quote! { bool },
        syn_solidity::Type::String(_) => quote! {
            alloc::alloc::String
        },
        syn_solidity::Type::Bytes(_) => quote! {
            pvm_contract_types::alloc::Bytes
        },
        syn_solidity::Type::FixedBytes(_, size) => {
            let size: usize = size.get().into();
            quote! {
                [u8; #size]
            }
        }
        syn_solidity::Type::Int(_, non_zero) => {
            let size = non_zero.unwrap().to_string();
            if size == "256" {
                return quote! { compile_error!("I256 is not implemented") };
            }
            let ident = format_ident!("i{}", size);
            quote! { #ident }
        }
        syn_solidity::Type::Uint(_, non_zero) => {
            let size = non_zero.unwrap().to_string();

            let mut ident = format!("u{}", size);
            if size == "256" {
                ident = capitalize(&ident);
            }
            let ident = format_ident!("{}", ident);
            quote! { #ident }
        }
        syn_solidity::Type::Tuple(type_tuple) => {
            let args = type_tuple.types.iter().map(to_rust_type);
            quote! {
                (#(#args),*)
            }
        }
        syn_solidity::Type::Array(type_array) => {
            let typ = to_rust_type(&type_array.ty);
            if let Some(size_lit) = type_array.size() {
                quote! {
                  [#typ; #size_lit]
                }
            } else {
                quote! {
                    alloc::vec::Vec<#typ>
                }
            }
        }
        typ @ syn_solidity::Type::Function(_) => {
            let lit = format!(
                "abi import for function types is not supported: {}",
                typ.to_string()
            );
            quote! {
                compile_error!(#lit);
            }
        }
        typ @ syn_solidity::Type::Mapping(_) => {
            let lit = format!(
                "abi import is not supported for type mapping: {}",
                typ.to_string()
            );
            quote! {
                compile_error!(#lit);
            }
        }
        syn_solidity::Type::Custom(_) => {
            let lit = format!(
                "abi import is not supported for custom types: {}",
                typ.to_string()
            );
            quote! {
                compile_error!(#lit);
            }
        }
    }
}

pub fn expand_to_module(file: &File) -> TokenStream {
    let modules = file.items.iter().filter_map(|item| match item {
        syn_solidity::Item::Contract(item_contract) if item_contract.is_interface() => {
            let contract_name = format_ident!("{}", capitalize(&item_contract.name.to_string()));
            let repr = format!("\n{}\n", item_contract.to_string());
            let funcs = item_contract
                .body
                .iter()
                .filter_map(|x| match x {
                    syn_solidity::Item::Function(x) => {
                        match x.kind {
                            syn_solidity::FunctionKind::Constructor(_) => Some((x,true)),
                            syn_solidity::FunctionKind::Function(_) => Some((x,false)),
                            syn_solidity::FunctionKind::Fallback(_) |
                            syn_solidity::FunctionKind::Receive(_) |
                            syn_solidity::FunctionKind::Modifier(_) => None,
                        }
                    },
                    _ => None,
                })
                .map(|(x, is_constructor)| expand_function(contract_name.clone(), x, is_constructor));
            Some(quote! {
                    #[doc = #repr]
                    #[derive(Clone, Copy)]
                    pub struct #contract_name<Mutability: StateMutability, Inputs: SolEncode,  Outputs: SolDecode, const INITIALIZED: bool> {
                        address: Address,
                        call_builder: CallBuilder<Mutability, Inputs, Outputs>
                    }

                    impl<Mutability: StateMutability, Inputs: SolEncode, Outputs: SolDecode> #contract_name<Mutability, Inputs, Outputs, false> {
                        #( #funcs )*
                    }

                    impl #contract_name<Pure, (), (), false> {
                        /// Create api for the contract from an address
                        pub fn from_address(address: Address) -> #contract_name<Pure, (), (), false> {
                            Self {
                                address,
                                call_builder: CallBuilder::<Pure, (), ()>::default()
                            }
                        }
                    }

                    impl<Mutability: StateMutability, Inputs: SolEncode, Outputs: SolDecode> #contract_name<Mutability, Inputs, Outputs, true> {
                        /// Set call limits for the given call
                        pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                            self.call_builder = self.call_builder.set_call_limits(limits);
                            self
                        }
                        /// Perform a call to another contract
                        pub fn call_raw(&self, input: &mut [u8], output: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.call(self.address, input, output)
                        }
                        /// Perform a delegated call to another contract
                        pub fn delegate_call_raw(&self, input: &mut [u8], output: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.delegate_call(self.address, input, output)
                        }
                    }

                    impl<Inputs: SolEncode, Outputs: SolDecode> #contract_name<Payable, Inputs, Outputs, true> {
                        /// Set the transfer `.value` of the call
                        pub fn set_value(mut self, value: u128) -> Self {
                            self.call_builder = self.call_builder.set_value(value);
                            self
                        }
                    }
            })
        }
        syn_solidity::Item::Contract(_)
        | syn_solidity::Item::Enum(_)
        | syn_solidity::Item::Error(_)
        | syn_solidity::Item::Event(_)
        | syn_solidity::Item::Function(_)
        | syn_solidity::Item::Import(_)
        | syn_solidity::Item::Pragma(_)
        | syn_solidity::Item::Struct(_)
        | syn_solidity::Item::Udt(_)
        | syn_solidity::Item::Using(_)
        | syn_solidity::Item::Variable(_) => None,
    });
    quote! {
        use pvm_contract_types::*;
        use pvm_contract_core::call::*;
        use ruint::ailiases::{U256};
        #(#modules)*
    }
}

#[cfg(test)]
mod test {
    use crate::abi_import::expand_to_module;
    use expect_test;
    use proc_macro2::Span;
    use quote::ToTokens;
    use syn::parse::{Parse, Parser};

    use super::load_json_abi;
    fn test_abi_contract_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples")
            .join("test-contracts")
            .join("target")
    }

    fn load(path: impl AsRef<std::path::Path>) -> String {
        let file = load_json_abi(
            "example".to_string(),
            Span::call_site(),
            &test_abi_contract_dir().join(path),
        )
        .unwrap();
        let tokens = expand_to_module(&file).to_token_stream();
        prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
    }

    #[test]
    fn multi_method() {
        let file = load("multi-method.release.abi.json");
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            use ruint::ailiases::U256;
            /**
            interface example {
                function add(uint256 a, uint256 b) external view returns (uint256);
                function getCounter() external view returns (uint256);
                function increment() external;
                function isZero(uint256 val) external view returns (bool);
                function mul(uint256 a, uint256 b) external view returns (uint256);
                function reset() external;
            }
            */
            #[derive(Clone, Copy)]
            pub struct Example<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
                const INITIALIZED: bool,
            > {
                address: Address,
                call_builder: CallBuilder<Mutability, Inputs, Outputs>,
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, false> {
                pub fn add(mut self, a: U256, b: U256) -> Example<View, (U256, U256), (U256), true> {
                    Example::<View, (U256, U256), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (U256, U256), (U256)> {
                            payload: (a, b),
                            selector: [119u8, 22u8, 2u8, 247u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_counter(mut self) -> Example<View, (), (U256), true> {
                    Example::<View, (), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (U256)> {
                            payload: (),
                            selector: [138u8, 218u8, 6u8, 110u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn increment(mut self) -> Example<NonPayable, (), (), true> {
                    Example::<NonPayable, (), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (), ()> {
                            payload: (),
                            selector: [208u8, 157u8, 224u8, 138u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn is_zero(mut self, val: U256) -> Example<View, (U256), (bool), true> {
                    Example::<View, (U256), (bool), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (U256), (bool)> {
                            payload: (val),
                            selector: [156u8, 43u8, 111u8, 192u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn mul(mut self, a: U256, b: U256) -> Example<View, (U256, U256), (U256), true> {
                    Example::<View, (U256, U256), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (U256, U256), (U256)> {
                            payload: (a, b),
                            selector: [200u8, 164u8, 172u8, 156u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn reset(mut self) -> Example<NonPayable, (), (), true> {
                    Example::<NonPayable, (), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (), ()> {
                            payload: (),
                            selector: [216u8, 38u8, 248u8, 143u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
            }
            impl Example<Pure, (), (), false> {
                /// Create api for the contract from an address
                pub fn from_address(address: Address) -> Example<Pure, (), (), false> {
                    Self {
                        address,
                        call_builder: CallBuilder::<Pure, (), ()>::default(),
                    }
                }
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, true> {
                /// Set call limits for the given call
                pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                    self.call_builder = self.call_builder.set_call_limits(limits);
                    self
                }
                /// Perform a call to another contract
                pub fn call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.call(self.address, input, output)
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.delegate_call(self.address, input, output)
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Set the transfer `.value` of the call
                pub fn set_value(mut self, value: u128) -> Self {
                    self.call_builder = self.call_builder.set_value(value);
                    self
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn storage_types() {
        let file = load("storage-types.release.abi.json");
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            use ruint::ailiases::U256;
            /**
            interface example {
                function getAddress() external view returns (address);
                function getBool() external view returns (bool);
                function getBytes32() external view returns (bytes32);
                function getU128() external view returns (uint128);
                function getU16() external view returns (uint16);
                function getU256() external view returns (uint256);
                function getU32() external view returns (uint32);
                function getU64() external view returns (uint64);
                function getU8() external view returns (uint8);
                function setAddress(address val) external;
                function setBool(bool val) external;
                function setBytes32(bytes32 val) external;
                function setU128(uint128 val) external;
                function setU16(uint16 val) external;
                function setU256(uint256 val) external;
                function setU32(uint32 val) external;
                function setU64(uint64 val) external;
                function setU8(uint8 val) external;
            }
            */
            #[derive(Clone, Copy)]
            pub struct Example<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
                const INITIALIZED: bool,
            > {
                address: Address,
                call_builder: CallBuilder<Mutability, Inputs, Outputs>,
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, false> {
                pub fn get_address(mut self) -> Example<View, (), (Address), true> {
                    Example::<View, (), (Address), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (Address)> {
                            payload: (),
                            selector: [56u8, 204u8, 72u8, 49u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_bool(mut self) -> Example<View, (), (bool), true> {
                    Example::<View, (), (bool), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (bool)> {
                            payload: (),
                            selector: [18u8, 167u8, 185u8, 20u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_bytes32(mut self) -> Example<View, (), ([u8; 32usize]), true> {
                    Example::<View, (), ([u8; 32usize]), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), ([u8; 32usize])> {
                            payload: (),
                            selector: [31u8, 144u8, 48u8, 55u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_u128(mut self) -> Example<View, (), (u128), true> {
                    Example::<View, (), (u128), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (u128)> {
                            payload: (),
                            selector: [148u8, 179u8, 108u8, 163u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_u16(mut self) -> Example<View, (), (u16), true> {
                    Example::<View, (), (u16), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (u16)> {
                            payload: (),
                            selector: [106u8, 0u8, 96u8, 206u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_u256(mut self) -> Example<View, (), (U256), true> {
                    Example::<View, (), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (U256)> {
                            payload: (),
                            selector: [56u8, 208u8, 249u8, 78u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_u32(mut self) -> Example<View, (), (u32), true> {
                    Example::<View, (), (u32), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (u32)> {
                            payload: (),
                            selector: [255u8, 105u8, 175u8, 182u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_u64(mut self) -> Example<View, (), (u64), true> {
                    Example::<View, (), (u64), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (u64)> {
                            payload: (),
                            selector: [170u8, 123u8, 219u8, 142u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_u8(mut self) -> Example<View, (), (u8), true> {
                    Example::<View, (), (u8), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (u8)> {
                            payload: (),
                            selector: [62u8, 72u8, 107u8, 252u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_address(
                    mut self,
                    val: Address,
                ) -> Example<NonPayable, (Address), (), true> {
                    Example::<NonPayable, (Address), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (Address), ()> {
                            payload: (val),
                            selector: [237u8, 18u8, 243u8, 6u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_bool(mut self, val: bool) -> Example<NonPayable, (bool), (), true> {
                    Example::<NonPayable, (bool), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (bool), ()> {
                            payload: (val),
                            selector: [13u8, 78u8, 4u8, 145u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_bytes32(
                    mut self,
                    val: [u8; 32usize],
                ) -> Example<NonPayable, ([u8; 32usize]), (), true> {
                    Example::<NonPayable, ([u8; 32usize]), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, ([u8; 32usize]), ()> {
                            payload: (val),
                            selector: [217u8, 75u8, 69u8, 66u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_u128(mut self, val: u128) -> Example<NonPayable, (u128), (), true> {
                    Example::<NonPayable, (u128), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (u128), ()> {
                            payload: (val),
                            selector: [106u8, 203u8, 223u8, 247u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_u16(mut self, val: u16) -> Example<NonPayable, (u16), (), true> {
                    Example::<NonPayable, (u16), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (u16), ()> {
                            payload: (val),
                            selector: [199u8, 21u8, 164u8, 153u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_u256(mut self, val: U256) -> Example<NonPayable, (U256), (), true> {
                    Example::<NonPayable, (U256), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (U256), ()> {
                            payload: (val),
                            selector: [21u8, 89u8, 64u8, 56u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_u32(mut self, val: u32) -> Example<NonPayable, (u32), (), true> {
                    Example::<NonPayable, (u32), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (u32), ()> {
                            payload: (val),
                            selector: [140u8, 193u8, 61u8, 176u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_u64(mut self, val: u64) -> Example<NonPayable, (u64), (), true> {
                    Example::<NonPayable, (u64), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (u64), ()> {
                            payload: (val),
                            selector: [247u8, 41u8, 141u8, 56u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn set_u8(mut self, val: u8) -> Example<NonPayable, (u8), (), true> {
                    Example::<NonPayable, (u8), (), true> {
                        address: self.address,
                        call_builder: CallBuilder::<NonPayable, (u8), ()> {
                            payload: (val),
                            selector: [148u8, 72u8, 210u8, 250u8],
                            witness: NonPayable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
            }
            impl Example<Pure, (), (), false> {
                /// Create api for the contract from an address
                pub fn from_address(address: Address) -> Example<Pure, (), (), false> {
                    Self {
                        address,
                        call_builder: CallBuilder::<Pure, (), ()>::default(),
                    }
                }
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, true> {
                /// Set call limits for the given call
                pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                    self.call_builder = self.call_builder.set_call_limits(limits);
                    self
                }
                /// Perform a call to another contract
                pub fn call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.call(self.address, input, output)
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.delegate_call(self.address, input, output)
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Set the transfer `.value` of the call
                pub fn set_value(mut self, value: u128) -> Self {
                    self.call_builder = self.call_builder.set_value(value);
                    self
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn return_values() {
        let file = load("return-values.release.abi.json");
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            use ruint::ailiases::U256;
            /**
            interface example {
                function getPair() external view returns (uint256, bool);
                function getTriple() external view returns (uint256, address, bool);
                function identity(uint256 val) external view returns (uint256);
            }
            */
            #[derive(Clone, Copy)]
            pub struct Example<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
                const INITIALIZED: bool,
            > {
                address: Address,
                call_builder: CallBuilder<Mutability, Inputs, Outputs>,
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, false> {
                pub fn get_pair(mut self) -> Example<View, (), (U256, bool), true> {
                    Example::<View, (), (U256, bool), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (U256, bool)> {
                            payload: (),
                            selector: [193u8, 241u8, 177u8, 181u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_triple(mut self) -> Example<View, (), (U256, Address, bool), true> {
                    Example::<View, (), (U256, Address, bool), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (U256, Address, bool)> {
                            payload: (),
                            selector: [72u8, 187u8, 245u8, 255u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn identity(mut self, val: U256) -> Example<View, (U256), (U256), true> {
                    Example::<View, (U256), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (U256), (U256)> {
                            payload: (val),
                            selector: [224u8, 131u8, 145u8, 91u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
            }
            impl Example<Pure, (), (), false> {
                /// Create api for the contract from an address
                pub fn from_address(address: Address) -> Example<Pure, (), (), false> {
                    Self {
                        address,
                        call_builder: CallBuilder::<Pure, (), ()>::default(),
                    }
                }
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, true> {
                /// Set call limits for the given call
                pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                    self.call_builder = self.call_builder.set_call_limits(limits);
                    self
                }
                /// Perform a call to another contract
                pub fn call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.call(self.address, input, output)
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.delegate_call(self.address, input, output)
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Set the transfer `.value` of the call
                pub fn set_value(mut self, value: u128) -> Self {
                    self.call_builder = self.call_builder.set_value(value);
                    self
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn composite_types() {
        let file = load("composite-types.release.abi.json");
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            use ruint::ailiases::U256;
            /**
            interface example {
                function getFixedArray() external view returns (uint256[3] memory);
                function processTuple((uint256,bool) data) external view returns (uint256);
                function sumFixedArray(uint256[3] memory scores) external view returns (uint256);
            }
            */
            #[derive(Clone, Copy)]
            pub struct Example<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
                const INITIALIZED: bool,
            > {
                address: Address,
                call_builder: CallBuilder<Mutability, Inputs, Outputs>,
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, false> {
                pub fn get_fixed_array(mut self) -> Example<View, (), ([U256; 3usize]), true> {
                    Example::<View, (), ([U256; 3usize]), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), ([U256; 3usize])> {
                            payload: (),
                            selector: [224u8, 203u8, 106u8, 154u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn process_tuple(
                    mut self,
                    data: (U256, bool),
                ) -> Example<View, ((U256, bool)), (U256), true> {
                    Example::<View, ((U256, bool)), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, ((U256, bool)), (U256)> {
                            payload: (data),
                            selector: [100u8, 29u8, 67u8, 90u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn sum_fixed_array(
                    mut self,
                    scores: [U256; 3usize],
                ) -> Example<View, ([U256; 3usize]), (U256), true> {
                    Example::<View, ([U256; 3usize]), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, ([U256; 3usize]), (U256)> {
                            payload: (scores),
                            selector: [74u8, 80u8, 202u8, 70u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
            }
            impl Example<Pure, (), (), false> {
                /// Create api for the contract from an address
                pub fn from_address(address: Address) -> Example<Pure, (), (), false> {
                    Self {
                        address,
                        call_builder: CallBuilder::<Pure, (), ()>::default(),
                    }
                }
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, true> {
                /// Set call limits for the given call
                pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                    self.call_builder = self.call_builder.set_call_limits(limits);
                    self
                }
                /// Perform a call to another contract
                pub fn call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.call(self.address, input, output)
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.delegate_call(self.address, input, output)
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Set the transfer `.value` of the call
                pub fn set_value(mut self, value: u128) -> Self {
                    self.call_builder = self.call_builder.set_value(value);
                    self
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn dynamic_types() {
        let file = load("dynamic-types.release.abi.json");
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            use ruint::ailiases::U256;
            /**
            interface example {
                function echoBytes() external view returns (bytes memory);
                function echoString() external view returns (string memory);
                function getArray() external view returns (uint256[] memory);
                function getBytesLength(bytes memory b) external view returns (uint256);
                function getStringLength(string memory s) external view returns (uint256);
                function sumArray(uint256[] memory arr) external view returns (uint256);
            }
            */
            #[derive(Clone, Copy)]
            pub struct Example<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
                const INITIALIZED: bool,
            > {
                address: Address,
                call_builder: CallBuilder<Mutability, Inputs, Outputs>,
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, false> {
                pub fn echo_bytes(
                    mut self,
                ) -> Example<View, (), (pvm_contract_types::alloc::Bytes), true> {
                    Example::<View, (), (pvm_contract_types::alloc::Bytes), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (pvm_contract_types::alloc::Bytes)> {
                            payload: (),
                            selector: [90u8, 98u8, 121u8, 241u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn echo_string(mut self) -> Example<View, (), (alloc::alloc::String), true> {
                    Example::<View, (), (alloc::alloc::String), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (alloc::alloc::String)> {
                            payload: (),
                            selector: [140u8, 104u8, 3u8, 183u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_array(mut self) -> Example<View, (), (alloc::vec::Vec<U256>), true> {
                    Example::<View, (), (alloc::vec::Vec<U256>), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (alloc::vec::Vec<U256>)> {
                            payload: (),
                            selector: [213u8, 4u8, 234u8, 29u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_bytes_length(
                    mut self,
                    b: pvm_contract_types::alloc::Bytes,
                ) -> Example<View, (pvm_contract_types::alloc::Bytes), (U256), true> {
                    Example::<View, (pvm_contract_types::alloc::Bytes), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<
                            View,
                            (pvm_contract_types::alloc::Bytes),
                            (U256),
                        > {
                            payload: (b),
                            selector: [43u8, 90u8, 36u8, 201u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_string_length(
                    mut self,
                    s: alloc::alloc::String,
                ) -> Example<View, (alloc::alloc::String), (U256), true> {
                    Example::<View, (alloc::alloc::String), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (alloc::alloc::String), (U256)> {
                            payload: (s),
                            selector: [159u8, 121u8, 99u8, 203u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn sum_array(
                    mut self,
                    arr: alloc::vec::Vec<U256>,
                ) -> Example<View, (alloc::vec::Vec<U256>), (U256), true> {
                    Example::<View, (alloc::vec::Vec<U256>), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (alloc::vec::Vec<U256>), (U256)> {
                            payload: (arr),
                            selector: [148u8, 196u8, 200u8, 237u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
            }
            impl Example<Pure, (), (), false> {
                /// Create api for the contract from an address
                pub fn from_address(address: Address) -> Example<Pure, (), (), false> {
                    Self {
                        address,
                        call_builder: CallBuilder::<Pure, (), ()>::default(),
                    }
                }
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, true> {
                /// Set call limits for the given call
                pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                    self.call_builder = self.call_builder.set_call_limits(limits);
                    self
                }
                /// Perform a call to another contract
                pub fn call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.call(self.address, input, output)
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.delegate_call(self.address, input, output)
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Set the transfer `.value` of the call
                pub fn set_value(mut self, value: u128) -> Self {
                    self.call_builder = self.call_builder.set_value(value);
                    self
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn constructor_args() {
        let file = load("constructor-args.release.abi.json");
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            use ruint::ailiases::U256;
            /**
            interface example {
                function getInitialSupply() external view returns (uint256);
                function getOwner() external view returns (address);
            }
            */
            #[derive(Clone, Copy)]
            pub struct Example<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
                const INITIALIZED: bool,
            > {
                address: Address,
                call_builder: CallBuilder<Mutability, Inputs, Outputs>,
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, false> {
                pub fn get_initial_supply(mut self) -> Example<View, (), (U256), true> {
                    Example::<View, (), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (U256)> {
                            payload: (),
                            selector: [129u8, 164u8, 166u8, 216u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                pub fn get_owner(mut self) -> Example<View, (), (Address), true> {
                    Example::<View, (), (Address), true> {
                        address: self.address,
                        call_builder: CallBuilder::<View, (), (Address)> {
                            payload: (),
                            selector: [137u8, 61u8, 32u8, 232u8],
                            witness: View::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
            }
            impl Example<Pure, (), (), false> {
                /// Create api for the contract from an address
                pub fn from_address(address: Address) -> Example<Pure, (), (), false> {
                    Self {
                        address,
                        call_builder: CallBuilder::<Pure, (), ()>::default(),
                    }
                }
            }
            impl<
                Mutability: StateMutability,
                Inputs: SolEncode,
                Outputs: SolDecode,
            > Example<Mutability, Inputs, Outputs, true> {
                /// Set call limits for the given call
                pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                    self.call_builder = self.call_builder.set_call_limits(limits);
                    self
                }
                /// Perform a call to another contract
                pub fn call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.call(self.address, input, output)
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input: &mut [u8],
                    output: &mut [u8],
                ) -> Result<Outputs, CallError> {
                    self.call_builder.delegate_call(self.address, input, output)
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Set the transfer `.value` of the call
                pub fn set_value(mut self, value: u128) -> Self {
                    self.call_builder = self.call_builder.set_value(value);
                    self
                }
            }
        "#]]
        .assert_eq(&file);
    }
}
