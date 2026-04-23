use ctxt::{Ctxt, compute_function_signature};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{self};
use syn_solidity::{File, ItemFunction, SolIdent};
pub mod parse;
use crate::signature::compute_selector;
use crate::solidity::{capitalize, to_pascal_case, to_snake_case};
mod ctxt;

pub fn expand_function(
    ctxt: &mut Ctxt,
    contract_name: syn::Ident,
    func: &ItemFunction,
    is_constructor: bool,
    alloc: bool,
) -> (bool, TokenStream) {
    let func_name = if is_constructor {
        format_ident!("{}_{}", "new", to_snake_case(&contract_name.to_string()))
    } else {
        format_ident!("{}", ctxt.function_name(func))
    };
    let selector: Vec<TokenStream> = if is_constructor {
        [0u8; 4].into_iter().map(|x| quote! { #x }).collect()
    } else {
        compute_selector(&compute_function_signature(func))
            .into_iter()
            .map(|x| quote! { #x })
            .collect()
    };
    let args = if func.parameters.is_empty() {
        quote! {}
    } else {
        let args = func.parameters.iter().enumerate().map(|(index, param)| {
            let typ = to_rust_type(&param.ty, alloc);
            let name = &param
                .name
                .as_ref()
                .unwrap_or(&SolIdent::new(&format!("s{}", index)))
                .to_string();
            let name = format_ident!("{}", to_snake_case(name));
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
        func.attributes
            .mutability()
            .map(|mutability| match mutability {
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
            })
            .unwrap_or_else(|| {
                quote! {
                    NonPayable
                }
            })
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
            alloc::string::String
        },
        syn_solidity::Type::Bytes(_) => quote! {
            pvm_contract_sdk::Bytes
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

pub fn expand_to_module(file: &File, alloc: bool, cdm: Option<&str>) -> TokenStream {
    let mut ctxt = Ctxt::default();
    ctxt.visit_file(file);
    let modules = file.items.iter().filter_map(|item| match item {
        syn_solidity::Item::Contract(item_contract) if item_contract.is_interface() => {
            let contract_name = format_ident!("{}", to_pascal_case(&item_contract.name.to_string()));
            let contract_module = format_ident!("{}", to_snake_case(&item_contract.name.to_string()));
            ctxt.set_ns(item_contract.name.clone());
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
                .map(|(x, is_constructor)| expand_function(&mut ctxt, contract_name.clone(), x, is_constructor, alloc));
            type Funcs = Vec<(bool, TokenStream)>;
            let (constructor, funcs): (Funcs, Funcs) = funcs.partition(|(is_constructor, _)| *is_constructor);
            let funcs = funcs.into_iter().map(|x| x.1);
            let constructor: Vec<TokenStream> = constructor.into_iter().map(|x| x.1).collect();
            let constructor = if constructor.is_empty() {
                quote! {}
            } else {
                quote! {
                    #(#constructor)*
                }
            };
            let alloc_calls = if alloc {
                quote! {
                        /// Perform a call to another contract
                        pub fn call(&self) -> Result<Outputs, CallError> {
                            let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![0; 4 + self.call_builder.payload.encode_len()];
                            self.call_builder.call_raw(self.address, input_buf.as_mut_slice())?;
                            let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![0; self.call_builder.output_size().max(512)];
                            self.call_builder.extract_output(output_buf.as_mut_slice())
                        }

                        /// Perform a delegated call to another contract
                        pub fn delegate_call(&self) -> Result<Outputs, CallError> {
                            let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![0; 4 + self.call_builder.payload.encode_len()];
                            self.call_builder.delegate_call_raw(self.address, input_buf.as_mut_slice())?;
                            let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![0; self.call_builder.output_size().max(512)];
                            self.call_builder.extract_output(output_buf.as_mut_slice())
                        }
                }
            } else {
                quote! {}
            };

            let alloc_instantiate = if alloc {
                quote! {
                        /// Instantiate another contract by it's code_hash
                        pub fn instantiate(&self, code_hash: &[u8;32], value: u128, limits: RefTimeAndProofSizeLimits, salt: Option<&[u8;32]>) -> Result<(Address, Outputs), CallError> {
                            let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![0; 32 + self.call_builder.payload.encode_len()];
                            let mut address_buf = [0u8; 20];
                            self.call_builder.instantiate_raw(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                            let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![0; self.call_builder.output_size().max(512)];
                            let output = self.call_builder.extract_output(output_buf.as_mut_slice())?;
                            Ok((address_buf.into(), output))
                        }
                }
            } else {
                quote! {}
            };
            let cdm_helper = generate_cdm_reference(&contract_name, cdm);

            Some(quote! {
                pub mod #contract_module {
                    use super::*;

                    #[derive(Clone, Copy)]
                    /// the code is derived from this interface
                    #[doc = #repr]
                    ///
                    pub struct #contract_name<Mutability: StateMutability, Inputs: SolEncode,  Outputs: SolDecode, const INITIALIZED: bool> {
                        address: Address,
                        call_builder: CallBuilder<Mutability, Inputs, Outputs>
                    }

                    #cdm_helper

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
                        pub fn call_raw(&self, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.call(self.address, input_buf, output_buf)
                        }
                        /// Perform a delegated call to another contract
                        pub fn delegate_call_raw(&self, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.delegate_call(self.address, input_buf, output_buf)
                        }

                        #alloc_calls
                    }

                    impl<Inputs: SolEncode, Outputs: SolDecode> #contract_name<Payable, Inputs, Outputs, true> {
                        /// Instantiate another contract by it's code_hash
                        pub fn instantiate_raw(&self, code_hash: &[u8;32], value: u128, limits: RefTimeAndProofSizeLimits, salt: Option<&[u8;32]>, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<(Address, Outputs), CallError> {
                            let mut address_buf = [0u8; 20];
                            let result = self.call_builder.instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf,
                                output_buf,
                                )?;
                            Ok((address_buf.into(), result))
                        }

                        #alloc_instantiate

                        /// Set the transfer `.value` of the call
                        pub fn set_value(mut self, value: u128) -> Self {
                            self.call_builder = self.call_builder.set_value(value);
                            self
                        }
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
        use pvm_contract_sdk::*;

        #(#modules)*
    }
}

/// Emit a `reference` submodule on the imported interface when `abi_import!`
/// specifies `cdm = "@ns/name"`. The submodule exposes two ways to build a
/// typed handle to the CDM-registered contract:
///
/// - `reference::lookup()` — resolves the address **at runtime** by calling
///   `ContractRegistry.getAddress(string)` on the registry deployed at
///   `CONTRACTS_REGISTRY_ADDR` (env var, baked at compile time). Late
///   binding; pays the registry roundtrip on every call.
///
/// - `reference::from_env()` — resolves the address **at compile time** by
///   looking up the CDM name in the `CDM_REGISTRY` env var, which is a
///   semicolon-delimited list of `name=hexaddress` pairs. The address is
///   baked into the binary as a literal — same end result as
///   `from_address(addr)`, but the address comes from the env instead of
///   being passed in at call sites.
fn generate_cdm_reference(contract_name: &syn::Ident, cdm: Option<&str>) -> TokenStream {
    let Some(cdm_name) = cdm else {
        return quote! {};
    };
    let selector = crate::signature::compute_selector("getAddress(string)");
    let [s0, s1, s2, s3] = selector;

    quote! {
        pub mod reference {
            use super::*;

            /// Address of the CDM contract registry, baked in from the
            /// `CONTRACTS_REGISTRY_ADDR` env var at compile time. Used by
            /// `lookup()`. Zero-address sentinel when unset — `lookup()`
            /// callers get a runtime revert rather than a build-time error,
            /// so contracts that only use `from_env()` still compile.
            const __CDM_REGISTRY_ADDR: [u8; 20] = {
                const fn hex(c: u8) -> u8 {
                    match c {
                        b'0'..=b'9' => c - b'0',
                        b'a'..=b'f' => c - b'a' + 10,
                        b'A'..=b'F' => c - b'A' + 10,
                        _ => panic!(
                            "CONTRACTS_REGISTRY_ADDR contains an invalid hex character"
                        ),
                    }
                }
                match option_env!("CONTRACTS_REGISTRY_ADDR") {
                    Some(s) => {
                        let b = s.as_bytes();
                        let off = if b.len() > 1
                            && b[0] == b'0'
                            && (b[1] == b'x' || b[1] == b'X')
                        {
                            2
                        } else {
                            0
                        };
                        assert!(
                            b.len() - off == 40,
                            "CONTRACTS_REGISTRY_ADDR must be 40 hex chars (optional 0x prefix)"
                        );
                        let mut r = [0u8; 20];
                        let mut i = 0;
                        while i < 20 {
                            r[i] = hex(b[off + i * 2]) << 4 | hex(b[off + i * 2 + 1]);
                            i += 1;
                        }
                        r
                    }
                    None => [0u8; 20],
                }
            };

            /// Address resolved at compile time from the `CDM_REGISTRY` env
            /// var. Zero-address sentinel when unset or the CDM name isn't
            /// present — `from_env()` panics at runtime in that case so
            /// contracts that only use `lookup()` still compile.
            const __CDM_FROM_ENV_ADDR: [u8; 20] = {
                const fn hex(c: u8) -> u8 {
                    match c {
                        b'0'..=b'9' => c - b'0',
                        b'a'..=b'f' => c - b'a' + 10,
                        b'A'..=b'F' => c - b'A' + 10,
                        _ => panic!("CDM_REGISTRY contains an invalid hex character"),
                    }
                }
                // Locate `<name>=` preceded by ';' or at the start of the
                // mapping. Returns the byte index of '=' on match.
                const fn find_entry(entries: &[u8], name: &[u8]) -> Option<usize> {
                    let mut i = 0;
                    while i + name.len() < entries.len() {
                        let at_start = i == 0 || entries[i - 1] == b';';
                        if at_start {
                            let mut j = 0;
                            while j < name.len() && entries[i + j] == name[j] {
                                j += 1;
                            }
                            if j == name.len() && entries[i + j] == b'=' {
                                return Some(i + j);
                            }
                        }
                        i += 1;
                    }
                    None
                }
                match option_env!("CDM_REGISTRY") {
                    Some(entries) => {
                        let bytes = entries.as_bytes();
                        let name = #cdm_name.as_bytes();
                        match find_entry(bytes, name) {
                            Some(eq) => {
                                let mut start = eq + 1;
                                // Tolerate optional `0x` prefix.
                                if start + 1 < bytes.len()
                                    && bytes[start] == b'0'
                                    && (bytes[start + 1] == b'x'
                                        || bytes[start + 1] == b'X')
                                {
                                    start += 2;
                                }
                                assert!(
                                    start + 40 <= bytes.len(),
                                    "CDM_REGISTRY entry is shorter than 40 hex chars"
                                );
                                let mut r = [0u8; 20];
                                let mut i = 0;
                                while i < 20 {
                                    r[i] = hex(bytes[start + i * 2]) << 4
                                        | hex(bytes[start + i * 2 + 1]);
                                    i += 1;
                                }
                                r
                            }
                            None => [0u8; 20],
                        }
                    }
                    None => [0u8; 20],
                }
            };

            /// Get a runtime-resolved interface to the CDM-registered
            /// contract. Calls `ContractRegistry.getAddress(string)` on the
            /// registry at `CONTRACTS_REGISTRY_ADDR`. Panics if the registry
            /// call fails or the contract is not registered.
            pub fn lookup() -> #contract_name<Pure, (), (), false> {
                extern crate alloc;

                let cdm_name: &str = #cdm_name;
                let name_len = cdm_name.len();
                let padded_len = name_len.div_ceil(32) * 32;

                let mut calldata = alloc::vec![0u8; 4 + 32 + 32 + padded_len];
                calldata[0] = #s0;
                calldata[1] = #s1;
                calldata[2] = #s2;
                calldata[3] = #s3;
                calldata[4 + 24..4 + 32].copy_from_slice(&(32u64).to_be_bytes());
                calldata[4 + 32 + 24..4 + 32 + 32]
                    .copy_from_slice(&(name_len as u64).to_be_bytes());
                calldata[4 + 64..4 + 64 + name_len].copy_from_slice(cdm_name.as_bytes());

                let mut output_buf = [0u8; 64];
                let mut output_ref: &mut [u8] = &mut output_buf[..];

                let result = PolkaVmHost::call_evm(
                    CallFlags::ALLOW_REENTRY,
                    &__CDM_REGISTRY_ADDR,
                    u64::MAX,
                    &[0u8; 32],
                    &calldata,
                    Some(&mut output_ref),
                );

                match result {
                    Ok(()) => {
                        let is_some = output_buf[31] != 0;
                        if !is_some {
                            panic!("CDM: contract not found in registry");
                        }
                        let mut addr = [0u8; 20];
                        addr.copy_from_slice(&output_buf[44..64]);
                        #contract_name::<Pure, (), (), false>::from_address(addr.into())
                    }
                    Err(_) => panic!("CDM: registry call failed"),
                }
            }

            /// Get a compile-time-resolved interface to the CDM-registered
            /// contract. The address is baked into the binary from the
            /// `CDM_REGISTRY` env var (`name=hex;name=hex;...`). Panics at
            /// runtime if the env var was unset at build time or did not
            /// contain an entry for the CDM name.
            pub fn from_env() -> #contract_name<Pure, (), (), false> {
                if __CDM_FROM_ENV_ADDR == [0u8; 20] {
                    panic!(concat!(
                        "from_env(): CDM_REGISTRY env var must be set at build time ",
                        "and contain an entry for `", #cdm_name, "=<hexaddress>`"
                    ));
                }
                #contract_name::<Pure, (), (), false>::from_address(
                    __CDM_FROM_ENV_ADDR.into(),
                )
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::abi_import::expand_to_module;
    use alloy_json_abi::ToSolConfig;
    use quote::ToTokens;
    use std::{fs, path::PathBuf};
    use syn::parse::{Parse, Parser};
    fn test_abi_contract_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("test_abi_contract")
    }

    fn cargo_load_abi(bin_name: &str) -> String {
        let dir = test_abi_contract_dir();

        fs::read_to_string(dir.join(format!("abi_{}.json", bin_name))).unwrap()
    }

    fn load(name: &str) -> String {
        load_with_cdm(name, None)
    }

    fn load_with_cdm(name: &str, cdm: Option<&str>) -> String {
        let name = &name.replace('-', "_");
        let json = cargo_load_abi(name);
        let parsed: alloy_json_abi::JsonAbi = serde_json::from_str(&json).unwrap();
        let config = ToSolConfig::new()
            .print_constructors(true)
            .for_sol_macro(true);

        let unparsed = &parsed.to_sol(name, Some(config));
        let tts = syn::parse_str::<proc_macro2::TokenStream>(unparsed).unwrap();

        let file = syn_solidity::parse2(quote::quote! {
            #tts
        })
        .unwrap();
        let tokens = expand_to_module(&file, true, cdm).to_token_stream();
        prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
    }

    /// `cdm = "..."` should emit the `reference` submodule with both
    /// `lookup` and `from_env` entry points, the registry-address and
    /// from-env-address consts, and the CDM name baked into the expansion.
    #[test]
    fn cdm_arg_emits_reference_submodule() {
        let code = load_with_cdm("multi-method", Some("@example/multi-method"));
        assert!(
            code.contains("pub mod reference"),
            "reference submodule should be emitted"
        );
        assert!(code.contains("pub fn lookup"), "lookup() should be emitted");
        assert!(
            code.contains("pub fn from_env"),
            "from_env() should be emitted"
        );
        assert!(
            code.contains("__CDM_REGISTRY_ADDR"),
            "registry-address const should be emitted"
        );
        assert!(
            code.contains("__CDM_FROM_ENV_ADDR"),
            "from-env-address const should be emitted"
        );
        assert!(
            code.contains("@example/multi-method"),
            "cdm name should be baked into the expansion"
        );
    }

    /// Without `cdm = "..."`, the `reference` submodule and its consts
    /// must not appear in the expansion.
    #[test]
    fn no_cdm_arg_omits_reference_submodule() {
        let code = load_with_cdm("multi-method", None);
        assert!(!code.contains("pub mod reference"));
        assert!(!code.contains("__CDM_REGISTRY_ADDR"));
        assert!(!code.contains("__CDM_FROM_ENV_ADDR"));
    }

    #[test]
    fn multi_method() {
        let file = load("multi-method");
        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod multi_method {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface multi_method {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                constructor() payable;
                function getCount() external payable returns (uint64);
                function setFlag(bool flag) external payable;
                function transfer(address to, uint256 amount, uint32 nonce) external payable returns (bool);
            }
            ```*/
                ///
                pub struct MultiMethod<
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
                > MultiMethod<Mutability, Inputs, Outputs, false> {
                    pub fn get_count(mut self) -> MultiMethod<Payable, (), (u64), true> {
                        MultiMethod::<Payable, (), (u64), true> {
                            address: self.address,
                            call_builder: CallBuilder::<Payable, (), (u64)> {
                                payload: (),
                                selector: [168u8, 125u8, 148u8, 44u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn set_flag(mut self, flag: bool) -> MultiMethod<Payable, (bool), (), true> {
                        MultiMethod::<Payable, (bool), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<Payable, (bool), ()> {
                                payload: (flag),
                                selector: [57u8, 39u8, 246u8, 175u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn transfer(
                        mut self,
                        to: Address,
                        amount: U256,
                        nonce: u32,
                    ) -> MultiMethod<Payable, (Address, U256, u32), (bool), true> {
                        MultiMethod::<Payable, (Address, U256, u32), (bool), true> {
                            address: self.address,
                            call_builder: CallBuilder::<Payable, (Address, U256, u32), (bool)> {
                                payload: (to, amount, nonce),
                                selector: [103u8, 215u8, 9u8, 208u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl MultiMethod<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(address: Address) -> MultiMethod<Pure, (), (), false> {
                        Self {
                            address,
                            call_builder: CallBuilder::<Pure, (), ()>::default(),
                        }
                    }
                }
                pub fn new_multi_method() -> MultiMethod<Payable, (), (), true> {
                    MultiMethod::<Payable, (), (), true> {
                        address: [0u8; 20].into(),
                        call_builder: CallBuilder::<Payable, (), ()> {
                            payload: (),
                            selector: [0u8, 0u8, 0u8, 0u8],
                            witness: Payable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > MultiMethod<Mutability, Inputs, Outputs, true> {
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
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call_raw(
                        &self,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract
                    pub fn call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.delegate_call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > MultiMethod<Payable, Inputs, Outputs, true> {
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate_raw(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut address_buf = [0u8; 20];
                        let result = self
                            .call_builder
                            .instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf,
                                output_buf,
                            )?;
                        Ok((address_buf.into(), result))
                    }
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        self.call_builder
                            .instantiate_raw(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        let output = self.call_builder.extract_output(output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn nested_custom_type() {
        let file = load("nested-custom-type");
        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod nested_custom_type {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface nested_custom_type {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                constructor() payable;
                function origin() external payable returns ((uint64,uint64) memory);
                function reflect(((uint64,uint64),(uint64,uint64)) memory line) external payable returns (((uint64,uint64),(uint64,uint64)) memory);
            }
            ```*/
                ///
                pub struct NestedCustomType<
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
                > NestedCustomType<Mutability, Inputs, Outputs, false> {
                    pub fn origin(mut self) -> NestedCustomType<Payable, (), ((u64, u64)), true> {
                        NestedCustomType::<Payable, (), ((u64, u64)), true> {
                            address: self.address,
                            call_builder: CallBuilder::<Payable, (), ((u64, u64))> {
                                payload: (),
                                selector: [147u8, 139u8, 95u8, 50u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn reflect(
                        mut self,
                        line: ((u64, u64), (u64, u64)),
                    ) -> NestedCustomType<
                        Payable,
                        (((u64, u64), (u64, u64))),
                        (((u64, u64), (u64, u64))),
                        true,
                    > {
                        NestedCustomType::<
                            Payable,
                            (((u64, u64), (u64, u64))),
                            (((u64, u64), (u64, u64))),
                            true,
                        > {
                            address: self.address,
                            call_builder: CallBuilder::<
                                Payable,
                                (((u64, u64), (u64, u64))),
                                (((u64, u64), (u64, u64))),
                            > {
                                payload: (line),
                                selector: [5u8, 150u8, 191u8, 142u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl NestedCustomType<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(address: Address) -> NestedCustomType<Pure, (), (), false> {
                        Self {
                            address,
                            call_builder: CallBuilder::<Pure, (), ()>::default(),
                        }
                    }
                }
                pub fn new_nested_custom_type() -> NestedCustomType<Payable, (), (), true> {
                    NestedCustomType::<Payable, (), (), true> {
                        address: [0u8; 20].into(),
                        call_builder: CallBuilder::<Payable, (), ()> {
                            payload: (),
                            selector: [0u8, 0u8, 0u8, 0u8],
                            witness: Payable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > NestedCustomType<Mutability, Inputs, Outputs, true> {
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
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call_raw(
                        &self,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract
                    pub fn call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.delegate_call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > NestedCustomType<Payable, Inputs, Outputs, true> {
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate_raw(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut address_buf = [0u8; 20];
                        let result = self
                            .call_builder
                            .instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf,
                                output_buf,
                            )?;
                        Ok((address_buf.into(), result))
                    }
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        self.call_builder
                            .instantiate_raw(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        let output = self.call_builder.extract_output(output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn composite_type_method() {
        let file = load("custom-type-method");
        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod custom_type_method {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface custom_type_method {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                constructor() payable;
                function touch((uint256,uint256) memory value) external payable returns ((uint256,uint256) memory);
            }
            ```*/
                ///
                pub struct CustomTypeMethod<
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
                > CustomTypeMethod<Mutability, Inputs, Outputs, false> {
                    pub fn touch(
                        mut self,
                        value: (U256, U256),
                    ) -> CustomTypeMethod<Payable, ((U256, U256)), ((U256, U256)), true> {
                        CustomTypeMethod::<Payable, ((U256, U256)), ((U256, U256)), true> {
                            address: self.address,
                            call_builder: CallBuilder::<Payable, ((U256, U256)), ((U256, U256))> {
                                payload: (value),
                                selector: [184u8, 219u8, 195u8, 2u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl CustomTypeMethod<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(address: Address) -> CustomTypeMethod<Pure, (), (), false> {
                        Self {
                            address,
                            call_builder: CallBuilder::<Pure, (), ()>::default(),
                        }
                    }
                }
                pub fn new_custom_type_method() -> CustomTypeMethod<Payable, (), (), true> {
                    CustomTypeMethod::<Payable, (), (), true> {
                        address: [0u8; 20].into(),
                        call_builder: CallBuilder::<Payable, (), ()> {
                            payload: (),
                            selector: [0u8, 0u8, 0u8, 0u8],
                            witness: Payable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > CustomTypeMethod<Mutability, Inputs, Outputs, true> {
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
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call_raw(
                        &self,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract
                    pub fn call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.delegate_call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > CustomTypeMethod<Payable, Inputs, Outputs, true> {
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate_raw(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut address_buf = [0u8; 20];
                        let result = self
                            .call_builder
                            .instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf,
                                output_buf,
                            )?;
                        Ok((address_buf.into(), result))
                    }
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        self.call_builder
                            .instantiate_raw(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        let output = self.call_builder.extract_output(output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn dynamic_custom_return() {
        let file = load("dynamic-custom-return");
        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod dynamic_custom_return {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface dynamic_custom_return {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                constructor() payable;
                function getNamed() external payable returns ((uint64,string) memory);
                function process((uint64,string) memory data, bool flag) external payable returns (uint64);
            }
            ```*/
                ///
                pub struct DynamicCustomReturn<
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
                > DynamicCustomReturn<Mutability, Inputs, Outputs, false> {
                    pub fn get_named(
                        mut self,
                    ) -> DynamicCustomReturn<Payable, (), ((u64, alloc::string::String)), true> {
                        DynamicCustomReturn::<Payable, (), ((u64, alloc::string::String)), true> {
                            address: self.address,
                            call_builder: CallBuilder::<
                                Payable,
                                (),
                                ((u64, alloc::string::String)),
                            > {
                                payload: (),
                                selector: [233u8, 148u8, 217u8, 223u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn process(
                        mut self,
                        data: (u64, alloc::string::String),
                        flag: bool,
                    ) -> DynamicCustomReturn<
                        Payable,
                        ((u64, alloc::string::String), bool),
                        (u64),
                        true,
                    > {
                        DynamicCustomReturn::<
                            Payable,
                            ((u64, alloc::string::String), bool),
                            (u64),
                            true,
                        > {
                            address: self.address,
                            call_builder: CallBuilder::<
                                Payable,
                                ((u64, alloc::string::String), bool),
                                (u64),
                            > {
                                payload: (data, flag),
                                selector: [57u8, 253u8, 73u8, 204u8],
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl DynamicCustomReturn<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(
                        address: Address,
                    ) -> DynamicCustomReturn<Pure, (), (), false> {
                        Self {
                            address,
                            call_builder: CallBuilder::<Pure, (), ()>::default(),
                        }
                    }
                }
                pub fn new_dynamic_custom_return() -> DynamicCustomReturn<Payable, (), (), true> {
                    DynamicCustomReturn::<Payable, (), (), true> {
                        address: [0u8; 20].into(),
                        call_builder: CallBuilder::<Payable, (), ()> {
                            payload: (),
                            selector: [0u8, 0u8, 0u8, 0u8],
                            witness: Payable::default(),
                            call_limits: Default::default(),
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > DynamicCustomReturn<Mutability, Inputs, Outputs, true> {
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
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call_raw(
                        &self,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract
                    pub fn call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.delegate_call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > DynamicCustomReturn<Payable, Inputs, Outputs, true> {
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate_raw(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut address_buf = [0u8; 20];
                        let result = self
                            .call_builder
                            .instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf,
                                output_buf,
                            )?;
                        Ok((address_buf.into(), result))
                    }
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        self.call_builder
                            .instantiate_raw(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        let output = self.call_builder.extract_output(output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn constructor_args() {
        let file = load("constructor-with-params");
        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod constructor_with_params {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface constructor_with_params {
                error CalldataTooLarge();
                error InvalidCalldata();
                error NoSelector();
                error UnknownSelector();
                constructor(address owner, uint256 supply) payable;
                function balanceOf(address account) external payable returns (uint256);
            }
            ```*/
                ///
                pub struct ConstructorWithParams<
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
                > ConstructorWithParams<Mutability, Inputs, Outputs, false> {
                    pub fn balance_of(
                        mut self,
                        account: Address,
                    ) -> ConstructorWithParams<Payable, (Address), (U256), true> {
                        ConstructorWithParams::<Payable, (Address), (U256), true> {
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
                impl ConstructorWithParams<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(
                        address: Address,
                    ) -> ConstructorWithParams<Pure, (), (), false> {
                        Self {
                            address,
                            call_builder: CallBuilder::<Pure, (), ()>::default(),
                        }
                    }
                }
                pub fn new_constructor_with_params(
                    owner: Address,
                    supply: U256,
                ) -> ConstructorWithParams<Payable, (Address, U256), (), true> {
                    ConstructorWithParams::<Payable, (Address, U256), (), true> {
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
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > ConstructorWithParams<Mutability, Inputs, Outputs, true> {
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
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call_raw(
                        &self,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract
                    pub fn call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.delegate_call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > ConstructorWithParams<Payable, Inputs, Outputs, true> {
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate_raw(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut address_buf = [0u8; 20];
                        let result = self
                            .call_builder
                            .instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf,
                                output_buf,
                            )?;
                        Ok((address_buf.into(), result))
                    }
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        self.call_builder
                            .instantiate_raw(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        let output = self.call_builder.extract_output(output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn overload() {
        let file = r#"
        [
            {
              "type": "function",
              "name": "flip",
              "inputs": [],
              "outputs": [],
              "stateMutability": "nonpayable"
            },
            {
              "type": "function",
              "name": "flip",
              "inputs": [
                {
                  "name": "data",
                  "type": "(uint64,string)"
                }
              ],
              "outputs": [],
              "stateMutability": "nonpayable"
            }
        ]
        "#;
        let file = {
            let parsed: alloy_json_abi::JsonAbi = serde_json::from_str(file).unwrap();
            let config = ToSolConfig::new()
                .print_constructors(true)
                .for_sol_macro(true);

            let unparsed = &parsed.to_sol("flipper", Some(config));
            let tts = syn::parse_str::<proc_macro2::TokenStream>(unparsed).unwrap();

            let file = syn_solidity::parse2(quote::quote! {
                #tts
            })
            .unwrap();
            let tokens = expand_to_module(&file, true, None).to_token_stream();
            prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
        };
        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod flipper {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface flipper {
                function flip() external;
                function flip((uint64,string) data) external;
            }
            ```*/
                ///
                pub struct Flipper<
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
                > Flipper<Mutability, Inputs, Outputs, false> {
                    pub fn flip_cde4efa9(mut self) -> Flipper<NonPayable, (), (), true> {
                        Flipper::<NonPayable, (), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<NonPayable, (), ()> {
                                payload: (),
                                selector: [205u8, 228u8, 239u8, 169u8],
                                witness: NonPayable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn flip_c353ca2b(
                        mut self,
                        data: (u64, alloc::string::String),
                    ) -> Flipper<NonPayable, ((u64, alloc::string::String)), (), true> {
                        Flipper::<NonPayable, ((u64, alloc::string::String)), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<
                                NonPayable,
                                ((u64, alloc::string::String)),
                                (),
                            > {
                                payload: (data),
                                selector: [195u8, 83u8, 202u8, 43u8],
                                witness: NonPayable::default(),
                                call_limits: Default::default(),
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl Flipper<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(address: Address) -> Flipper<Pure, (), (), false> {
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
                > Flipper<Mutability, Inputs, Outputs, true> {
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
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call_raw(
                        &self,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract
                    pub fn call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                    /// Perform a delegated call to another contract
                    pub fn delegate_call(&self) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.delegate_call_raw(self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        self.call_builder.extract_output(output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> Flipper<Payable, Inputs, Outputs, true> {
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate_raw(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut address_buf = [0u8; 20];
                        let result = self
                            .call_builder
                            .instantiate(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf,
                                output_buf,
                            )?;
                        Ok((address_buf.into(), result))
                    }
                    /// Instantiate another contract by it's code_hash
                    pub fn instantiate(
                        &self,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        self.call_builder
                            .instantiate_raw(
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size().max(512)
                        ];
                        let output = self.call_builder.extract_output(output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
            }
        "#]].assert_eq(&file);
    }
}
