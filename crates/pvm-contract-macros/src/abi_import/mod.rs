use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{self};
use syn_solidity::{File, ItemFunction, SolIdent};
pub mod parse;
use crate::signature::compute_selector;
use crate::solidity::{capitalize, to_snake_case};

pub fn expand_function(
    contract_name: syn::Ident,
    func: &ItemFunction,
    is_constructor: bool,
    alloc: bool,
) -> (bool, TokenStream) {
    let func_name = if is_constructor {
        format_ident!("{}_{}", "new",  to_snake_case(&contract_name.to_string()))
    } else {
        format_ident!("{}", to_snake_case(&func.name().to_string()))
    };
    let selector: Vec<TokenStream> = if is_constructor {
        [0u8; 4].into_iter().map(|x| quote! { #x }).collect()
    } else {
        let mut name = format!("{}{}", func.name(), func.call_type());
        if name.rfind(",").is_some_and(|x| x == name.len() - 2) {
            name.remove(name.len() - 2);
        }
        compute_selector(&name)
            .into_iter()
            .map(|x| quote! { #x })
            .collect()
    };
    let args = if func.parameters.is_empty() {
        quote! {}
    } else {
        let args = func.parameters.iter().map(|param| {
            let typ = to_rust_type(&param.ty, alloc);
            let name = &param
                .name
                .as_ref()
                .unwrap_or(&SolIdent::new("s"))
                .to_string();
            let name = format_ident!("{}", to_snake_case(&name));
            quote! {#name: #typ}
        });
        quote! { #(#args),* }
    };

    let return_type = if let Some(ret) = func.return_type() {
        let typ = to_rust_type(&ret, alloc);
        quote! { #typ}
    } else {
        quote! { () }
    };

    let self_ = if is_constructor {
        quote! {}
    } else {
        quote! {mut self, }
    };

    let types = func.parameters.types().map(|x| to_rust_type(x, alloc));
    let names = func.parameters.names().map(|name| {
        let name = name.as_ref().map_or(&SolIdent::new("s"), |v| v).to_string();
        format_ident!("{}", name)
    });

    let state_mutability = if is_constructor {
        quote! {
            Payable
        }
    } else {
        if let Some(mutability) = func.attributes.mutability() {
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
        }
    };
    let types: Vec<TokenStream> = types.collect();
    let address = if is_constructor {
        quote! {[0u8;20].into()}
    } else {
        quote! { self.address }
    };
    let res = quote! {
        pub fn #func_name(#self_ #args) -> #contract_name<#state_mutability, ( #(#types),* ), #return_type, true> {
            #contract_name::<#state_mutability, ( #(#types),* ), #return_type, true> {
                address: #address,
                call_builder: CallBuilder::<#state_mutability, ( #(#types),* ), #return_type> {
                    payload: (#(#names),*),
                    selector: [#(#selector),*],
                    witness: #state_mutability::default(),
                    call_limits: Default::default(),
                    _ret: core::marker::PhantomData,
                }
            }
        }
    };
    (is_constructor, res)
}

fn to_rust_type(typ: &syn_solidity::Type, alloc: bool) -> TokenStream {
    if !alloc && typ.is_abi_dynamic() {
        return quote! {
            compile_error!("Enable alloc to support dynamic types")
        };
    }
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
            let mut ident = format!("i{}", size);
            if size == "256" {
                ident = capitalize(&ident);
            }
            let ident = format_ident!("{}", ident);
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
            let args = type_tuple.types.iter().map(|x| to_rust_type(x, alloc));
            quote! {
                (#(#args),*)
            }
        }
        syn_solidity::Type::Array(type_array) => {
            let typ = to_rust_type(&type_array.ty, alloc);
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
            let lit = format!("abi import for function types is not supported: {}", typ);
            quote! {
                compile_error!(#lit);
            }
        }
        typ @ syn_solidity::Type::Mapping(_) => {
            let lit = format!("abi import is not supported for type mapping: {}", typ);
            quote! {
                compile_error!(#lit);
            }
        }
        syn_solidity::Type::Custom(_) => {
            let lit = format!("abi import is not supported for custom types: {}", typ);
            quote! {
                compile_error!(#lit);
            }
        }
    }
}

pub fn expand_to_module(file: &File, alloc: bool) -> TokenStream {
    let modules = file.items.iter().filter_map(|item| match item {
        syn_solidity::Item::Contract(item_contract) if item_contract.is_interface() => {
            let contract_name = format_ident!("{}", capitalize(&item_contract.name.to_string()));
            let contract_module = format_ident!("{}", to_snake_case(&item_contract.name.to_string()));

            let repr = format!("```solidity\n{}\n```", item_contract);
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
                .map(|(x, is_constructor)| expand_function(contract_name.clone(), x, is_constructor, alloc));
            let (constructor, funcs): (Vec<(bool, TokenStream)>, Vec<(bool, TokenStream)>) = funcs.partition(|(is_constructor, _)| *is_constructor);
            let funcs = funcs.into_iter().map(|x| x.1);
            let constructor: Vec<TokenStream> = constructor.into_iter().map(|x| x.1).collect();
            let constructor = if constructor.is_empty() {
                quote! {}
            } else {
                quote! {
                    pub mod #contract_module {
                        use super::*;
                        #(#constructor)*
                    }
                }
            };
            Some(quote! {
                    #[derive(Clone, Copy)]
                    /// the code is derived from this interface
                    #[doc = #repr]
                    ///
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

                    #constructor
                    

                    impl<Mutability: StateMutability, Inputs: SolEncode, Outputs: SolDecode> #contract_name<Mutability, Inputs, Outputs, true> {
                        /// Set call limits for the given call
                        pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                            self.call_builder = self.call_builder.set_call_limits(limits);
                            self
                        }
                        /// Perform a call to another contract
                        pub fn call_raw(&self, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, errors::Error> {
                            self.call_builder.call(self.address, input_buf, output_buf).map_err(|e| errors::Error::from(e))
                        }
                        /// Perform a delegated call to another contract
                        pub fn delegate_call_raw(&self, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, errors::Error> {
                            self.call_builder.delegate_call(self.address, input_buf, output_buf).map_err(|e| errors::Error::from(e))
                        }
                    }

                    impl<Inputs: SolEncode, Outputs: SolDecode> #contract_name<Payable, Inputs, Outputs, true> {
                        /// Instantiate another contract by it's code_hash
                        pub fn instantiate_raw(&self, code_hash: &[u8;32], value: u128, limits: RefTimeAndProofSizeLimits, salt: Option<&[u8;32]>, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<(Address, Outputs), errors::Error> {
                            let mut address_buf = [0u8; 20];
                            let result = self.call_builder.instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                input_buf,
                                &mut address_buf,
                                output_buf,
                                ).map_err(|e| errors::Error::from(e))?;
                            Ok((address_buf.into(), result))
                        }
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

        pub mod errors {
            use super::*; 

            #[derive(pvm_contract_macros::SolError)]
            struct CalldataTooLarge;

            #[derive(pvm_contract_macros::SolError)]
            struct InvalidCalldata;

            #[derive(pvm_contract_macros::SolError)]
            struct NoSelector;

            #[derive(pvm_contract_macros::SolError)]
            struct UnknownSelector;

            sol_revert_enum! {
                pub enum Error {
                    CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata),
                    NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector),
                    CallError(CallError),
                }
            }
        }

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

    use super::parse::load_json_abi;
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
        let tokens = expand_to_module(&file, true).to_token_stream();
        prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
    }

    #[test]
    fn multi_method() {
        let file = load("multi-method.release.abi.json");
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            pub mod errors {
                use super::*;
                #[derive(pvm_contract_macros::SolError)]
                struct CalldataTooLarge;
                #[derive(pvm_contract_macros::SolError)]
                struct InvalidCalldata;
                #[derive(pvm_contract_macros::SolError)]
                struct NoSelector;
                #[derive(pvm_contract_macros::SolError)]
                struct UnknownSelector;
                sol_revert_enum! {
                    pub enum Error { CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata), NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector), CallError(CallError), }
                }
            }
            #[derive(Clone, Copy)]
            /// the code is derived from this interface
            /**```solidity
            interface example {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                function add(uint256 a, uint256 b) external view returns (uint256);
                function getCounter() external view returns (uint256);
                function increment() external;
                function isZero(uint256 val) external view returns (bool);
                function mul(uint256 a, uint256 b) external view returns (uint256);
                function reset() external;
            }
            ```*/
            ///
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
                            selector: [122u8, 56u8, 249u8, 235u8],
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
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .delegate_call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Instantiate another contract by it's code_hash
                pub fn instantiate_raw(
                    &self,
                    code_hash: &[u8; 32],
                    value: u128,
                    limits: RefTimeAndProofSizeLimits,
                    salt: Option<&[u8; 32]>,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<(Address, Outputs), errors::Error> {
                    let mut address_buf = [0u8; 20];
                    let result = self
                        .call_builder
                        .instantiate(
                            limits,
                            value,
                            code_hash,
                            salt,
                            input_buf,
                            &mut address_buf,
                            output_buf,
                        )
                        .map_err(|e| errors::Error::from(e))?;
                    Ok((address_buf.into(), result))
                }
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
            pub mod errors {
                use super::*;
                #[derive(pvm_contract_macros::SolError)]
                struct CalldataTooLarge;
                #[derive(pvm_contract_macros::SolError)]
                struct InvalidCalldata;
                #[derive(pvm_contract_macros::SolError)]
                struct NoSelector;
                #[derive(pvm_contract_macros::SolError)]
                struct UnknownSelector;
                sol_revert_enum! {
                    pub enum Error { CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata), NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector), CallError(CallError), }
                }
            }
            #[derive(Clone, Copy)]
            /// the code is derived from this interface
            /**```solidity
            interface example {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
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
            ```*/
            ///
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
                            selector: [227u8, 0u8, 129u8, 160u8],
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
                            selector: [30u8, 38u8, 253u8, 51u8],
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
                            selector: [194u8, 177u8, 42u8, 115u8],
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
                            selector: [211u8, 14u8, 187u8, 114u8],
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
                            selector: [38u8, 78u8, 92u8, 151u8],
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
                            selector: [213u8, 98u8, 193u8, 230u8],
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
                            selector: [188u8, 72u8, 142u8, 235u8],
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
                            selector: [50u8, 201u8, 139u8, 121u8],
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
                            selector: [23u8, 185u8, 13u8, 148u8],
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
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .delegate_call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Instantiate another contract by it's code_hash
                pub fn instantiate_raw(
                    &self,
                    code_hash: &[u8; 32],
                    value: u128,
                    limits: RefTimeAndProofSizeLimits,
                    salt: Option<&[u8; 32]>,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<(Address, Outputs), errors::Error> {
                    let mut address_buf = [0u8; 20];
                    let result = self
                        .call_builder
                        .instantiate(
                            limits,
                            value,
                            code_hash,
                            salt,
                            input_buf,
                            &mut address_buf,
                            output_buf,
                        )
                        .map_err(|e| errors::Error::from(e))?;
                    Ok((address_buf.into(), result))
                }
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
            pub mod errors {
                use super::*;
                #[derive(pvm_contract_macros::SolError)]
                struct CalldataTooLarge;
                #[derive(pvm_contract_macros::SolError)]
                struct InvalidCalldata;
                #[derive(pvm_contract_macros::SolError)]
                struct NoSelector;
                #[derive(pvm_contract_macros::SolError)]
                struct UnknownSelector;
                sol_revert_enum! {
                    pub enum Error { CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata), NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector), CallError(CallError), }
                }
            }
            #[derive(Clone, Copy)]
            /// the code is derived from this interface
            /**```solidity
            interface example {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                function getPair() external view returns (uint256, bool);
                function getTriple() external view returns (uint256, address, bool);
                function identity(uint256 val) external view returns (uint256);
            }
            ```*/
            ///
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
                            selector: [172u8, 55u8, 238u8, 187u8],
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
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .delegate_call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Instantiate another contract by it's code_hash
                pub fn instantiate_raw(
                    &self,
                    code_hash: &[u8; 32],
                    value: u128,
                    limits: RefTimeAndProofSizeLimits,
                    salt: Option<&[u8; 32]>,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<(Address, Outputs), errors::Error> {
                    let mut address_buf = [0u8; 20];
                    let result = self
                        .call_builder
                        .instantiate(
                            limits,
                            value,
                            code_hash,
                            salt,
                            input_buf,
                            &mut address_buf,
                            output_buf,
                        )
                        .map_err(|e| errors::Error::from(e))?;
                    Ok((address_buf.into(), result))
                }
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
            pub mod errors {
                use super::*;
                #[derive(pvm_contract_macros::SolError)]
                struct CalldataTooLarge;
                #[derive(pvm_contract_macros::SolError)]
                struct InvalidCalldata;
                #[derive(pvm_contract_macros::SolError)]
                struct NoSelector;
                #[derive(pvm_contract_macros::SolError)]
                struct UnknownSelector;
                sol_revert_enum! {
                    pub enum Error { CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata), NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector), CallError(CallError), }
                }
            }
            #[derive(Clone, Copy)]
            /// the code is derived from this interface
            /**```solidity
            interface example {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                function getFixedArray() external view returns (uint256[3] memory);
                function processTuple((uint256,bool) data) external view returns (uint256);
                function sumFixedArray(uint256[3] memory scores) external view returns (uint256);
            }
            ```*/
            ///
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
                            selector: [19u8, 194u8, 222u8, 227u8],
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
                            selector: [7u8, 166u8, 156u8, 213u8],
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
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .delegate_call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Instantiate another contract by it's code_hash
                pub fn instantiate_raw(
                    &self,
                    code_hash: &[u8; 32],
                    value: u128,
                    limits: RefTimeAndProofSizeLimits,
                    salt: Option<&[u8; 32]>,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<(Address, Outputs), errors::Error> {
                    let mut address_buf = [0u8; 20];
                    let result = self
                        .call_builder
                        .instantiate(
                            limits,
                            value,
                            code_hash,
                            salt,
                            input_buf,
                            &mut address_buf,
                            output_buf,
                        )
                        .map_err(|e| errors::Error::from(e))?;
                    Ok((address_buf.into(), result))
                }
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
            pub mod errors {
                use super::*;
                #[derive(pvm_contract_macros::SolError)]
                struct CalldataTooLarge;
                #[derive(pvm_contract_macros::SolError)]
                struct InvalidCalldata;
                #[derive(pvm_contract_macros::SolError)]
                struct NoSelector;
                #[derive(pvm_contract_macros::SolError)]
                struct UnknownSelector;
                sol_revert_enum! {
                    pub enum Error { CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata), NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector), CallError(CallError), }
                }
            }
            #[derive(Clone, Copy)]
            /// the code is derived from this interface
            /**```solidity
            interface example {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                function echoBytes() external view returns (bytes memory);
                function echoString() external view returns (string memory);
                function getArray() external view returns (uint256[] memory);
                function getBytesLength(bytes memory b) external view returns (uint256);
                function getStringLength(string memory s) external view returns (uint256);
                function sumArray(uint256[] memory arr) external view returns (uint256);
            }
            ```*/
            ///
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
                            selector: [128u8, 54u8, 240u8, 103u8],
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
                            selector: [101u8, 193u8, 154u8, 240u8],
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
                            selector: [30u8, 42u8, 234u8, 6u8],
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
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .delegate_call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Instantiate another contract by it's code_hash
                pub fn instantiate_raw(
                    &self,
                    code_hash: &[u8; 32],
                    value: u128,
                    limits: RefTimeAndProofSizeLimits,
                    salt: Option<&[u8; 32]>,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<(Address, Outputs), errors::Error> {
                    let mut address_buf = [0u8; 20];
                    let result = self
                        .call_builder
                        .instantiate(
                            limits,
                            value,
                            code_hash,
                            salt,
                            input_buf,
                            &mut address_buf,
                            output_buf,
                        )
                        .map_err(|e| errors::Error::from(e))?;
                    Ok((address_buf.into(), result))
                }
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
            pub mod errors {
                use super::*;
                #[derive(pvm_contract_macros::SolError)]
                struct CalldataTooLarge;
                #[derive(pvm_contract_macros::SolError)]
                struct InvalidCalldata;
                #[derive(pvm_contract_macros::SolError)]
                struct NoSelector;
                #[derive(pvm_contract_macros::SolError)]
                struct UnknownSelector;
                sol_revert_enum! {
                    pub enum Error { CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata), NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector), CallError(CallError), }
                }
            }
            #[derive(Clone, Copy)]
            /// the code is derived from this interface
            /**```solidity
            interface example {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                function getInitialSupply() external view returns (uint256);
                function getOwner() external view returns (address);
            }
            ```*/
            ///
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
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .delegate_call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Instantiate another contract by it's code_hash
                pub fn instantiate_raw(
                    &self,
                    code_hash: &[u8; 32],
                    value: u128,
                    limits: RefTimeAndProofSizeLimits,
                    salt: Option<&[u8; 32]>,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<(Address, Outputs), errors::Error> {
                    let mut address_buf = [0u8; 20];
                    let result = self
                        .call_builder
                        .instantiate(
                            limits,
                            value,
                            code_hash,
                            salt,
                            input_buf,
                            &mut address_buf,
                            output_buf,
                        )
                        .map_err(|e| errors::Error::from(e))?;
                    Ok((address_buf.into(), result))
                }
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
    fn constructor() {
        let file = load(
            "../../../crates/pvm-contract-macros/tests/test_abi_contract/abi_constructor_with_params.json",
        );
        expect_test::expect![[r#"
            use pvm_contract_types::*;
            use pvm_contract_core::call::*;
            pub mod errors {
                use super::*;
                #[derive(pvm_contract_macros::SolError)]
                struct CalldataTooLarge;
                #[derive(pvm_contract_macros::SolError)]
                struct InvalidCalldata;
                #[derive(pvm_contract_macros::SolError)]
                struct NoSelector;
                #[derive(pvm_contract_macros::SolError)]
                struct UnknownSelector;
                sol_revert_enum! {
                    pub enum Error { CalldataTooLarge(CalldataTooLarge),
                    InvalidCalldata(InvalidCalldata), NoSelector(NoSelector),
                    UnknownSelector(UnknownSelector), CallError(CallError), }
                }
            }
            #[derive(Clone, Copy)]
            /// the code is derived from this interface
            /**```solidity
            interface example {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                constructor(address owner, uint256 supply) payable;
                function balanceOf(address account) external payable returns (uint256);
            }
            ```*/
            ///
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
                pub fn balance_of(
                    mut self,
                    account: Address,
                ) -> Example<Payable, (Address), (U256), true> {
                    Example::<Payable, (Address), (U256), true> {
                        address: self.address,
                        call_builder: CallBuilder::<Payable, (Address), (U256)> {
                            payload: (account),
                            selector: [112u8, 160u8, 130u8, 49u8],
                            witness: Payable::default(),
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
            pub mod example {
                use super::*;
                pub fn new_example(
                    owner: Address,
                    supply: U256,
                ) -> Example<Payable, (Address, U256), (), true> {
                    Example::<Payable, (Address, U256), (), true> {
                        address: [0u8; 20].into(),
                        call_builder: CallBuilder::<Payable, (Address, U256), ()> {
                            payload: (owner, supply),
                            selector: [0u8, 0u8, 0u8, 0u8],
                            witness: Payable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
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
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
                /// Perform a delegated call to another contract
                pub fn delegate_call_raw(
                    &self,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<Outputs, errors::Error> {
                    self.call_builder
                        .delegate_call(self.address, input_buf, output_buf)
                        .map_err(|e| errors::Error::from(e))
                }
            }
            impl<Inputs: SolEncode, Outputs: SolDecode> Example<Payable, Inputs, Outputs, true> {
                /// Instantiate another contract by it's code_hash
                pub fn instantiate_raw(
                    &self,
                    code_hash: &[u8; 32],
                    value: u128,
                    limits: RefTimeAndProofSizeLimits,
                    salt: Option<&[u8; 32]>,
                    input_buf: &mut [u8],
                    output_buf: &mut [u8],
                ) -> Result<(Address, Outputs), errors::Error> {
                    let mut address_buf = [0u8; 20];
                    let result = self
                        .call_builder
                        .instantiate(
                            limits,
                            value,
                            code_hash,
                            salt,
                            input_buf,
                            &mut address_buf,
                            output_buf,
                        )
                        .map_err(|e| errors::Error::from(e))?;
                    Ok((address_buf.into(), result))
                }
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
