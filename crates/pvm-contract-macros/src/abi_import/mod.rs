use ctxt::{Ctxt, Resolution};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn_solidity::{File, ItemFunction, SolIdent, Spanned};
pub mod parse;
use crate::utils::{build_method_signature_expr, capitalize, to_pascal_case, to_snake_case};
mod ctxt;

pub fn expand_function(
    ctxt: &mut Ctxt,
    contract_name: syn::Ident,
    func: &ItemFunction,
    is_constructor: bool,
    alloc: bool,
) -> syn::Result<(bool, TokenStream)> {
    let func_name = if is_constructor {
        format_ident!("{}_{}", "new", to_snake_case(&contract_name.to_string()))
    } else {
        format_ident!("{}", ctxt.function_name(func))
    };
    let (param_types, return_type) = resolve_signature_types(func, alloc, ctxt)?;
    let state_mutability = state_mutability_type(func, is_constructor)?;

    let names = func.parameters.names().enumerate().map(|(index, name)| {
        let name = name
            .as_ref()
            .map_or_else(|| format!("s{index}"), |v| v.to_string());
        format_ident!("{}", to_snake_case(&name))
    });

    let selector: TokenStream = if is_constructor {
        let sel: Vec<TokenStream> = [0u8; 4].into_iter().map(|x| quote! { #x }).collect();
        quote! {[#(#sel),*]}
    } else {
        let sig_expr = build_method_signature_expr(&func.name().to_string(), &param_types);

        quote! {
            const {
                ::pvm_contract_sdk::const_selector(#sig_expr)
            }
        }
    };
    let args = if func.parameters.is_empty() {
        quote! {}
    } else {
        let params = func.parameters.iter().enumerate().zip(&param_types);
        let args = params.map(|((index, param), typ)| {
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

    let self_ = if is_constructor {
        quote! {}
    } else {
        quote! {mut self, }
    };

    let address = if is_constructor {
        quote! {[0u8;20].into()}
    } else {
        quote! { self.address }
    };
    let res = quote! {
        pub fn #func_name(#self_ #args) -> #contract_name<#state_mutability, ( #(#param_types),* ), #return_type, true> {
            #contract_name::<#state_mutability, ( #(#param_types),* ), #return_type, true> {
                address: #address,
                call_builder: CallBuilder::<#state_mutability, ( #(#param_types),* ), #return_type> {
                    payload: (#(#names),*),
                    selector: #selector,
                    witness: #state_mutability::default(),
                    call_limits: Default::default(),
                    allow_reentry: false,
                    _ret: core::marker::PhantomData,
                }
            }
        }
    };
    Ok((is_constructor, res))
}

/// Resolve every type in `func`'s signature to its Rust spelling, exactly
/// once — the results are spliced into several positions of the expansion.
/// Failures are combined so each offending type gets its own spanned
/// diagnostic, and any failure aborts the whole item: a partially resolved
/// parameter list would silently shorten the canonical signature the selector
/// hashes and misalign the argument names against their types.
fn resolve_signature_types(
    func: &ItemFunction,
    alloc: bool,
    ctxt: &mut Ctxt,
) -> syn::Result<(Vec<syn::Type>, TokenStream)> {
    let param_types = try_collect_combined(func.parameters.types().map(|x| {
        to_rust_type(x, alloc, ctxt).map(|typ| {
            // An `Ok` type is always parseable; the error arms return `Err`.
            syn::parse2::<syn::Type>(typ.clone())
                .unwrap_or_else(|e| panic!("invalid rust type generated;\nerror: {e}\ntype:{typ}"))
        })
    }));
    let return_type = match func.return_type() {
        Some(ret) => to_rust_type(&ret, alloc, ctxt).map(|typ| quote! { #typ}),
        None => Ok(quote! { () }),
    };
    combine_results(param_types, return_type)
}

/// The `StateMutability` marker type for `func`. Solidity's pre-0.5
/// `constant` mutability is unsupported.
fn state_mutability_type(func: &ItemFunction, is_constructor: bool) -> syn::Result<TokenStream> {
    if is_constructor {
        return Ok(quote! { Payable });
    }
    Ok(match func.attributes.mutability() {
        Some(syn_solidity::Mutability::Pure(_)) => quote! { Pure },
        Some(syn_solidity::Mutability::View(_)) => quote! { View },
        Some(syn_solidity::Mutability::Payable(_)) => quote! { Payable },
        Some(mutability @ syn_solidity::Mutability::Constant(_)) => {
            return Err(syn::Error::new(
                mutability.span(),
                "constant mutability no supported",
            ));
        }
        None => quote! { NonPayable },
    })
}

/// Collect results, combining all errors so each offending element gets its
/// own spanned diagnostic; any failure fails the whole collection
/// (all-or-nothing — a partially resolved list would misalign siblings).
fn try_collect_combined<T>(iter: impl Iterator<Item = syn::Result<T>>) -> syn::Result<Vec<T>> {
    let mut err: Option<syn::Error> = None;
    let mut ok = Vec::new();
    for item in iter {
        match item {
            Ok(v) => ok.push(v),
            Err(e) => match &mut err {
                Some(acc) => acc.combine(e),
                None => err = Some(e),
            },
        }
    }
    match err {
        Some(err) => Err(err),
        None => Ok(ok),
    }
}

/// Combine two independent results, merging both errors so the first failure
/// doesn't mask diagnostics from the second.
fn combine_results<A, B>(a: syn::Result<A>, b: syn::Result<B>) -> syn::Result<(A, B)> {
    match (a, b) {
        (Ok(a), Ok(b)) => Ok((a, b)),
        (Err(mut e), Err(e2)) => {
            e.combine(e2);
            Err(e)
        }
        (Err(e), Ok(_)) | (Ok(_), Err(e)) => Err(e),
    }
}

/// Resolve a Solidity type to its Rust spelling.
///
/// Failure is returned rather than expanded to a `compile_error!` in place:
/// callers splice the result into several positions, and an inline error would
/// be reported once per splice.
fn to_rust_type(
    typ: &syn_solidity::Type,
    alloc: bool,
    ctxt: &mut Ctxt,
) -> syn::Result<TokenStream> {
    // `ctxt.is_abi_dynamic` resolves user-defined types (enum → `uint8`,
    // UDT → underlying, struct → its fields); syn-solidity's own
    // `Type::is_abi_dynamic` would reject every custom type as dynamic.
    if !alloc && ctxt.is_abi_dynamic(typ) {
        return Err(syn::Error::new(
            typ.span(),
            "Enable alloc to support dynamic types",
        ));
    }
    Ok(match typ {
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
            let size = non_zero.map(|x| x.get()).unwrap_or(256u16).to_string();
            let mut ident = format!("i{}", size);
            if size == "256" {
                ident = capitalize(&ident);
            }
            let ident = format_ident!("{}", ident);
            quote! { #ident }
        }
        syn_solidity::Type::Uint(_, non_zero) => {
            let size = non_zero.map(|x| x.get()).unwrap_or(256u16).to_string();

            let mut ident = format!("u{}", size);
            if size == "256" {
                ident = capitalize(&ident);
            }
            let ident = format_ident!("{}", ident);
            quote! { #ident }
        }
        syn_solidity::Type::Tuple(type_tuple) => {
            let args = type_tuple
                .types
                .iter()
                .map(|x| to_rust_type(x, alloc, ctxt))
                .collect::<syn::Result<Vec<TokenStream>>>()?;
            quote! {
                (#(#args),*)
            }
        }
        syn_solidity::Type::Array(type_array) => {
            let typ = to_rust_type(&type_array.ty, alloc, ctxt)?;
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
        syn_solidity::Type::Custom(custom) => {
            let name = format_ident!("{}", to_pascal_case(&custom.last().to_string()));
            match ctxt.resolve(custom)? {
                Resolution::Local => quote! { #name },
                Resolution::TopLevel => quote! { super::#name },
                // Interface modules are siblings at the invocation site: from
                // inside one, another is reached via `super::`; from a
                // file-level item (spliced directly at the invocation site) a
                // bare path suffices — the sibling module shadows any glob
                // import by language rule, and unlike `self::` it also
                // resolves for function-local items.
                Resolution::Qualified { ns, from_interface } => {
                    let ns = format_ident!("{}", to_snake_case(&ns.to_string()));
                    if from_interface {
                        quote! { super::#ns::#name }
                    } else {
                        quote! { #ns::#name }
                    }
                }
            }
        }
        typ @ syn_solidity::Type::Function(_) => {
            let lit = format!("abi import for function types is not supported: {}", typ);
            return Err(syn::Error::new(typ.span(), lit));
        }
        typ @ syn_solidity::Type::Mapping(_) => {
            let lit = format!("abi import is not supported for type mapping: {}", typ);
            return Err(syn::Error::new(typ.span(), lit));
        }
    })
}

/// Expand struct/error fields to `pub name: Type` declarations, one combined
/// diagnostic per unresolvable field type.
fn expand_fields<'a>(
    fields: impl Iterator<Item = &'a syn_solidity::VariableDeclaration>,
    ctxt: &mut Ctxt,
    alloc: bool,
) -> syn::Result<Vec<TokenStream>> {
    try_collect_combined(fields.enumerate().map(|(idx, x)| {
        let name = format_ident!(
            "{}",
            to_snake_case(
                &x.name
                    .clone()
                    .map(|x| x.as_string())
                    .unwrap_or(format!("param_{}", idx))
            )
        );
        to_rust_type(&x.ty, alloc, ctxt).map(|typ| {
            quote! {
                pub #name: #typ
            }
        })
    }))
}

fn expand_struct(
    x: &syn_solidity::ItemStruct,
    ctxt: &mut Ctxt,
    alloc: bool,
) -> syn::Result<TokenStream> {
    let fields = expand_fields(x.fields.iter(), ctxt, alloc)?;
    let name = format_ident!("{}", to_pascal_case(&x.name.to_string()));
    Ok(quote! {
        #[derive(SolType, PartialEq, Eq,  Debug)]
        pub struct #name {
            #(#fields),*
        }
    })
}

fn expand_error(
    x: &syn_solidity::ItemError,
    ctxt: &mut Ctxt,
    alloc: bool,
) -> syn::Result<TokenStream> {
    let fields = expand_fields(x.parameters.iter(), ctxt, alloc)?;
    let name = format_ident!("{}", to_pascal_case(&x.name.to_string()));
    Ok(quote! {
        #[derive(SolError, PartialEq, Eq, Debug)]
        pub struct #name {
            #(#fields),*
        }
    })
}

fn expand_udt(x: &syn_solidity::ItemUdt, ctxt: &mut Ctxt, alloc: bool) -> syn::Result<TokenStream> {
    let name = format_ident!("{}", to_pascal_case(&x.name.to_string()));
    let typ = to_rust_type(&x.ty, alloc, ctxt)?;
    let sol_typ = x.ty.abi_name();
    Ok(quote! {
        #[derive(PartialEq, Eq, Debug)]
        pub struct #name(pub #typ);

        impl From<#typ> for #name {
            fn from(value: #typ) -> #name {
                #name(value)
            }
        }

        impl From<#name> for #typ {
            fn from(value: #name) -> #typ {
                value.0
            }
        }

        impl SolEncode for #name {
            const IS_DYNAMIC: bool = false;
            const SOL_NAME: &'static str = #sol_typ;

            #[inline]
            fn encode_body_len(&self) -> usize {
                32
            }

            fn encode_body_to(&self, buf: &mut [u8]) {
                #typ::encode_body_to(&self.0, buf)
            }
        }

        impl StaticEncodedLen for #name {
            const ENCODED_SIZE: usize = 32;
        }

        impl SolDecode for #name {
            fn decode_at(input: &[u8], offset: usize) -> Result<#name, DecodeError> {
                #typ::decode_at(input, offset).map(|x| x.into())
            }
        }

        impl StaticDecode for #name {
            unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self {
                unsafe { #typ::decode_unchecked(input, offset).into() }
            }
        }
    })
}
fn expand_enum(x: &syn_solidity::ItemEnum) -> TokenStream {
    let name = format_ident!("{}", to_pascal_case(&x.name.to_string()));
    let variants = x
        .variants
        .iter()
        .map(|x| format_ident!("{}", x.ident.to_string()));
    let res = quote! {
        #[derive(PartialEq, Eq, Debug, ::pvm_contract_sdk::SolType)]
        #[repr(u8)]
        pub enum #name {
            #(#variants),*
        }
    };
    res
}
fn expand_items<'a>(
    items: impl Iterator<Item = &'a syn_solidity::Item>,
    alloc: bool,
    ctxt: &mut Ctxt,
) -> impl Iterator<Item = TokenStream> {
    items.filter_map(move |x| {
        let expanded = match x {
            syn_solidity::Item::Struct(x) => expand_struct(x, ctxt, alloc),
            syn_solidity::Item::Error(x) => expand_error(x, ctxt, alloc),
            syn_solidity::Item::Udt(x) => expand_udt(x, ctxt, alloc),
            syn_solidity::Item::Enum(x) => Ok(expand_enum(x)),
            _ => return None,
        };
        // Conversion boundary (per type item): the error replaces just this
        // item's slot, so sibling items keep expanding. The other two sites
        // are the funcs closure in `expand_to_module` (per function) and
        // `abi_import` in lib.rs (whole-output failures).
        Some(expanded.unwrap_or_else(|e| e.to_compile_error()))
    })
}

pub fn expand_to_module(file: &File, alloc: bool) -> syn::Result<TokenStream> {
    // Whole-invocation failures carry no better span than the invocation
    // itself: on stable, `file.span()` degrades to the first item's span
    // (`Span::join` is unavailable), which would misattribute a whole-file
    // error (e.g. a duplicate name between items 3 and 7) to item 1.
    crate::utils::reject_sol_imports(file)
        .map_err(|e| syn::Error::new(proc_macro2::Span::call_site(), e))?;
    let mut ctxt = Ctxt::default();
    ctxt.visit_file(file)
        .map_err(|e| syn::Error::new(proc_macro2::Span::call_site(), e))?;
    let modules = file.items.iter().filter_map(|item| match item {
        syn_solidity::Item::Contract(item_contract) if item_contract.is_interface() => {
            let contract_name = format_ident!("{}", to_pascal_case(&item_contract.name.to_string()));
            let contract_module = format_ident!("{}", to_snake_case(&item_contract.name.to_string()));
            ctxt.with_ns(item_contract.name.clone(), |ctxt: &mut Ctxt| {
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
                .map(|(x, is_constructor)| {
                    // Conversion boundary (per function): the error replaces
                    // just this function, so sibling items keep expanding.
                    // The other two sites are `expand_items` (per type item)
                    // and `abi_import` in lib.rs (whole-output failures).
                    expand_function(ctxt, contract_name.clone(), x, is_constructor, alloc)
                        .unwrap_or_else(|e| (is_constructor, e.to_compile_error()))
                });
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
            // Per-mutability `alloc_calls`: View/Pure callees borrow the
            // contract root immutably (`&R0`), so they can be invoked from
            // `&self` (view) caller methods. NonPayable/Payable callees
            // require `&mut R0`, so the borrow checker rejects invocations
            // from `&self` methods.
            //
            // `delegate_call` always takes `&mut R0` regardless of callee
            // mutability — the callee runs in caller's storage context, so
            // even a "view" callee can mutate caller state.
            let alloc_calls_readonly = if alloc {
                quote! {
                        /// Perform a call to another contract.
                        pub fn call<R0: ContractContext>(&self, root: &R0) -> Result<Outputs, CallError> {
                            let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![0; 4 + self.call_builder.payload.encode_len()];
                            self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                            let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![0; self.call_builder.output_size(root.host()).max(512)];
                            self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                        }
                }
            } else {
                quote! {}
            };
            let alloc_calls_mutating = if alloc {
                quote! {
                        /// Perform a call to another contract.
                        pub fn call<R0: ContractContext>(&self, root: &mut R0) -> Result<Outputs, CallError> {
                            let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![0; 4 + self.call_builder.payload.encode_len()];
                            // Clone the host handle before the mutable borrow:
                            // `Host` is a ZST on riscv64 (free) and `Rc` on
                            // host-target builds (refcount bump). Removes the
                            // need to re-borrow `root` for `extract_output`.
                            let host = root.host().clone();
                            self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                            let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![0; self.call_builder.output_size(&host).max(512)];
                            self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                        }
                }
            } else {
                quote! {}
            };
            let alloc_delegate = if alloc {
                quote! {
                        /// Perform a delegated call to another contract.
                        pub fn delegate_call<R0: ContractContext>(&self, root: &mut R0) -> Result<Outputs, CallError> {
                            let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![0; 4 + self.call_builder.payload.encode_len()];
                            let host = root.host().clone();
                            self.call_builder.delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                            let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![0; self.call_builder.output_size(&host).max(512)];
                            self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                        }
                }
            } else {
                quote! {}
            };

            let alloc_instantiate = if alloc {
                quote! {
                        /// Instantiate another contract by it's code_hash
                        pub fn instantiate<R0: ContractContext>(&self, root: &mut R0, code_hash: &[u8;32], value: u128, limits: RefTimeAndProofSizeLimits, salt: Option<&[u8;32]>) -> Result<(Address, Outputs), CallError> {
                            let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![0; 32 + self.call_builder.payload.encode_len()];
                            let mut address_buf = [0u8; 20];
                            let host = root.host().clone();
                            self.call_builder.instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                            let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![0; self.call_builder.output_size(&host).max(512)];
                            let output = self.call_builder.extract_output(&host, output_buf.as_mut_slice())?;
                            Ok((address_buf.into(), output))
                        }
                }
            } else {
                quote! {}
            };

            let user_types = expand_items(item_contract
            .body
            .iter(),alloc, ctxt);


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
                        /// Set call limits for the given call.
                        pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                            self.call_builder = self.call_builder.set_call_limits(limits);
                            self
                        }

                        /// Perform a delegated call to another contract.
                        ///
                        /// Always requires `&mut impl ContractContext` regardless of the
                        /// callee's declared mutability: the callee runs in caller's
                        /// storage context, so even a "view" callee can mutate state.
                        pub fn delegate_call_raw<R0: ContractContext>(&self, root: &mut R0, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                        }

                        #alloc_delegate
                    }

                    // View / Pure callees: callable from `&self` (read-only) caller methods.
                    impl<Inputs: SolEncode, Outputs: SolDecode> #contract_name<View, Inputs, Outputs, true> {
                        /// Perform a call to a `view` callee.
                        pub fn call_raw<R0: ContractContext>(&self, root: &R0, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.call(root, self.address, input_buf, output_buf)
                        }

                        #alloc_calls_readonly
                    }
                    impl<Inputs: SolEncode, Outputs: SolDecode> #contract_name<Pure, Inputs, Outputs, true> {
                        /// Perform a call to a `pure` callee.
                        pub fn call_raw<R0: ContractContext>(&self, root: &R0, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.call(root, self.address, input_buf, output_buf)
                        }

                        #alloc_calls_readonly
                    }

                    // NonPayable / Payable callees: require `&mut self` caller.
                    impl<Inputs: SolEncode, Outputs: SolDecode> #contract_name<NonPayable, Inputs, Outputs, true> {
                        /// Perform a call to a `nonpayable` callee. Caller must take
                        /// `&mut self` — `&self` (view) caller methods cannot construct
                        /// the `&mut impl ContractContext` argument.
                        pub fn call_raw<R0: ContractContext>(&self, root: &mut R0, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.call(root, self.address, input_buf, output_buf)
                        }

                        #alloc_calls_mutating
                    }
                    impl<Inputs: SolEncode, Outputs: SolDecode> #contract_name<Payable, Inputs, Outputs, true> {
                        /// Perform a call to a `payable` callee. Caller must take
                        /// `&mut self`.
                        pub fn call_raw<R0: ContractContext>(&self, root: &mut R0, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<Outputs, CallError> {
                            self.call_builder.call(root, self.address, input_buf, output_buf)
                        }

                        #alloc_calls_mutating

                        /// Instantiate another contract by it's code_hash. Always
                        /// requires `&mut impl ContractContext`: instantiation transfers
                        /// value, emits a deploy event, and bumps the caller's nonce.
                        pub fn instantiate_raw<R0: ContractContext>(&self, root: &mut R0, code_hash: &[u8;32], value: u128, limits: RefTimeAndProofSizeLimits, salt: Option<&[u8;32]>, input_buf: &mut [u8], output_buf: &mut [u8]) -> Result<(Address, Outputs), CallError> {
                            let mut address_buf = [0u8; 20];
                            let result = self.call_builder.instantiate(
                                root,
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

                        /// Set the transfer `.value` of the call.
                        pub fn set_value(mut self, value: u128) -> Self {
                            self.call_builder = self.call_builder.set_value(value);
                            self
                        }
                    }

                    #(#user_types)*
                }
            })
        })
        }
        syn_solidity::Item::Contract(_)
        | syn_solidity::Item::Enum(_)
        | syn_solidity::Item::Error(_)
        | syn_solidity::Item::Event(_)
        | syn_solidity::Item::Function(_)
        | syn_solidity::Item::Import(_)
        | syn_solidity::Item::Pragma(_)
        | syn_solidity::Item::Udt(_)
        | syn_solidity::Item::Struct(_)
        | syn_solidity::Item::Using(_)
        | syn_solidity::Item::Variable(_) => None,
    }).collect::<Vec<TokenStream>>();

    let user_types = expand_items(file.items.iter(), alloc, &mut ctxt);

    Ok(quote! {
        use pvm_contract_sdk::*;

        #(#modules)*
        #(#user_types)*
    })
}

#[cfg(test)]
mod test {
    use crate::abi_import::expand_to_module;
    use alloy_json_abi::ToSolConfig;
    use quote::{ToTokens, quote};
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
        let tokens = expand_to_module(&file, true).unwrap().to_token_stream();
        prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
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
                error NonPayableValueReceived();
                error UnknownSelector();
                constructor();
                function add(uint64 a, uint64 b) external pure returns (uint64);
                function deposit() external payable;
                function getCount() external view returns (uint64);
                function setFlag(bool flag) external;
                function transfer(address to, uint256 amount, uint32 nonce) external returns (bool);
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
                    pub fn add(
                        mut self,
                        a: u64,
                        b: u64,
                    ) -> MultiMethod<Pure, (u64, u64), (u64), true> {
                        MultiMethod::<Pure, (u64, u64), (u64), true> {
                            address: self.address,
                            call_builder: CallBuilder::<Pure, (u64, u64), (u64)> {
                                payload: (a, b),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "add(", < u64 as ::pvm_contract_sdk::SolEncode > ::SOL_NAME,
                                            ",", < u64 as ::pvm_contract_sdk::SolEncode > ::SOL_NAME,
                                            ")"
                                        ),
                                    )
                                },
                                witness: Pure::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn deposit(mut self) -> MultiMethod<Payable, (), (), true> {
                        MultiMethod::<Payable, (), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<Payable, (), ()> {
                                payload: (),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!("deposit(", ")"),
                                    )
                                },
                                witness: Payable::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn get_count(mut self) -> MultiMethod<View, (), (u64), true> {
                        MultiMethod::<View, (), (u64), true> {
                            address: self.address,
                            call_builder: CallBuilder::<View, (), (u64)> {
                                payload: (),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!("getCount(", ")"),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn set_flag(
                        mut self,
                        flag: bool,
                    ) -> MultiMethod<NonPayable, (bool), (), true> {
                        MultiMethod::<NonPayable, (bool), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<NonPayable, (bool), ()> {
                                payload: (flag),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "setFlag(", < bool as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: NonPayable::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn transfer(
                        mut self,
                        to: Address,
                        amount: U256,
                        nonce: u32,
                    ) -> MultiMethod<NonPayable, (Address, U256, u32), (bool), true> {
                        MultiMethod::<NonPayable, (Address, U256, u32), (bool), true> {
                            address: self.address,
                            call_builder: CallBuilder::<NonPayable, (Address, U256, u32), (bool)> {
                                payload: (to, amount, nonce),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "transfer(", < Address as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ",", < U256 as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ",", < u32 as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: NonPayable::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
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
                            allow_reentry: false,
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > MultiMethod<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > MultiMethod<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > MultiMethod<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > MultiMethod<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > MultiMethod<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct CalldataTooLarge {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct InvalidCalldata {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NoSelector {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NonPayableValueReceived {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct UnknownSelector {}
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
                error NonPayableValueReceived();
                error UnknownSelector();
                constructor();
                function origin() external view returns ((uint64,uint64) memory);
                function reflect(((uint64,uint64),(uint64,uint64)) memory line) external view returns (((uint64,uint64),(uint64,uint64)) memory);
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
                    pub fn origin(mut self) -> NestedCustomType<View, (), ((u64, u64)), true> {
                        NestedCustomType::<View, (), ((u64, u64)), true> {
                            address: self.address,
                            call_builder: CallBuilder::<View, (), ((u64, u64))> {
                                payload: (),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!("origin(", ")"),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn reflect(
                        mut self,
                        line: ((u64, u64), (u64, u64)),
                    ) -> NestedCustomType<
                        View,
                        (((u64, u64), (u64, u64))),
                        (((u64, u64), (u64, u64))),
                        true,
                    > {
                        NestedCustomType::<
                            View,
                            (((u64, u64), (u64, u64))),
                            (((u64, u64), (u64, u64))),
                            true,
                        > {
                            address: self.address,
                            call_builder: CallBuilder::<
                                View,
                                (((u64, u64), (u64, u64))),
                                (((u64, u64), (u64, u64))),
                            > {
                                payload: (line),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "reflect(", < ((u64, u64), (u64, u64)) as
                                            ::pvm_contract_sdk::SolEncode > ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
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
                            allow_reentry: false,
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > NestedCustomType<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > NestedCustomType<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > NestedCustomType<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > NestedCustomType<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > NestedCustomType<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct CalldataTooLarge {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct InvalidCalldata {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NoSelector {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NonPayableValueReceived {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct UnknownSelector {}
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
                error NonPayableValueReceived();
                error UnknownSelector();
                constructor();
                function touch((uint256,uint256) memory value) external view returns ((uint256,uint256) memory);
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
                    ) -> CustomTypeMethod<View, ((U256, U256)), ((U256, U256)), true> {
                        CustomTypeMethod::<View, ((U256, U256)), ((U256, U256)), true> {
                            address: self.address,
                            call_builder: CallBuilder::<View, ((U256, U256)), ((U256, U256))> {
                                payload: (value),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "touch(", < (U256, U256) as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
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
                            allow_reentry: false,
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > CustomTypeMethod<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > CustomTypeMethod<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > CustomTypeMethod<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > CustomTypeMethod<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > CustomTypeMethod<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct CalldataTooLarge {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct InvalidCalldata {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NoSelector {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NonPayableValueReceived {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct UnknownSelector {}
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
                error NonPayableValueReceived();
                error UnknownSelector();
                constructor();
                function getNamed() external view returns ((uint64,string) memory);
                function process((uint64,string) memory data, bool flag) external view returns (uint64);
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
                    ) -> DynamicCustomReturn<View, (), ((u64, alloc::string::String)), true> {
                        DynamicCustomReturn::<View, (), ((u64, alloc::string::String)), true> {
                            address: self.address,
                            call_builder: CallBuilder::<View, (), ((u64, alloc::string::String))> {
                                payload: (),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!("getNamed(", ")"),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn process(
                        mut self,
                        data: (u64, alloc::string::String),
                        flag: bool,
                    ) -> DynamicCustomReturn<
                        View,
                        ((u64, alloc::string::String), bool),
                        (u64),
                        true,
                    > {
                        DynamicCustomReturn::<
                            View,
                            ((u64, alloc::string::String), bool),
                            (u64),
                            true,
                        > {
                            address: self.address,
                            call_builder: CallBuilder::<
                                View,
                                ((u64, alloc::string::String), bool),
                                (u64),
                            > {
                                payload: (data, flag),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "process(", < (u64, alloc::string::String) as
                                            ::pvm_contract_sdk::SolEncode > ::SOL_NAME, ",", < bool as
                                            ::pvm_contract_sdk::SolEncode > ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
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
                            allow_reentry: false,
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > DynamicCustomReturn<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > DynamicCustomReturn<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > DynamicCustomReturn<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > DynamicCustomReturn<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > DynamicCustomReturn<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct CalldataTooLarge {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct InvalidCalldata {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NoSelector {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NonPayableValueReceived {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct UnknownSelector {}
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
                error NonPayableValueReceived();
                error UnknownSelector();
                constructor(address owner, uint256 supply);
                function balanceOf(address account) external view returns (uint256);
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
                    ) -> ConstructorWithParams<View, (Address), (U256), true> {
                        ConstructorWithParams::<View, (Address), (U256), true> {
                            address: self.address,
                            call_builder: CallBuilder::<View, (Address), (U256)> {
                                payload: (account),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "balanceOf(", < Address as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
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
                            allow_reentry: false,
                            _ret: core::marker::PhantomData,
                        },
                    }
                }
                impl<
                    Mutability: StateMutability,
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > ConstructorWithParams<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > ConstructorWithParams<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > ConstructorWithParams<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > ConstructorWithParams<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > ConstructorWithParams<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct CalldataTooLarge {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct InvalidCalldata {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NoSelector {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct NonPayableValueReceived {}
                #[derive(SolError, PartialEq, Eq, Debug)]
                pub struct UnknownSelector {}
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn point() {
        let file = quote! {
                error Test(string str);

                type Example is uint256;

                struct Point {
                    uint a;
                    uint b;
                }

                interface Ballot {
                    struct Voter { // Struct
                        uint weight;
                        bool voted;
                        address delegate;
                        uint vote;
                    }

                    function sendVoterInfo(Voter voter) external;
                    function add(Point a, Point b) external;
                }
        };
        let file = {
            let file = syn_solidity::parse2(file).unwrap();
            let tokens = expand_to_module(&file, true).unwrap().to_token_stream();
            prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
        };
        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod ballot {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface Ballot {
                struct Voter { uint weight; bool voted; address delegate; uint vote; }
                function sendVoterInfo(Voter voter) external;
                function add(Point a, Point b) external;
            }
            ```*/
                ///
                pub struct Ballot<
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
                > Ballot<Mutability, Inputs, Outputs, false> {
                    pub fn send_voter_info(
                        mut self,
                        voter: Voter,
                    ) -> Ballot<NonPayable, (Voter), (), true> {
                        Ballot::<NonPayable, (Voter), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<NonPayable, (Voter), ()> {
                                payload: (voter),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "sendVoterInfo(", < Voter as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: NonPayable::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                    pub fn add(
                        mut self,
                        a: super::Point,
                        b: super::Point,
                    ) -> Ballot<NonPayable, (super::Point, super::Point), (), true> {
                        Ballot::<NonPayable, (super::Point, super::Point), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<
                                NonPayable,
                                (super::Point, super::Point),
                                (),
                            > {
                                payload: (a, b),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "add(", < super::Point as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ",", < super::Point as
                                            ::pvm_contract_sdk::SolEncode > ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: NonPayable::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl Ballot<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(address: Address) -> Ballot<Pure, (), (), false> {
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
                > Ballot<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> Ballot<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> Ballot<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > Ballot<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> Ballot<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(SolType, PartialEq, Eq, Debug)]
                pub struct Voter {
                    pub weight: U256,
                    pub voted: bool,
                    pub delegate: Address,
                    pub vote: U256,
                }
            }
            #[derive(SolError, PartialEq, Eq, Debug)]
            pub struct Test {
                pub str: alloc::string::String,
            }
            #[derive(PartialEq, Eq, Debug)]
            pub struct Example(pub U256);
            impl From<U256> for Example {
                fn from(value: U256) -> Example {
                    Example(value)
                }
            }
            impl From<Example> for U256 {
                fn from(value: Example) -> U256 {
                    value.0
                }
            }
            impl SolEncode for Example {
                const IS_DYNAMIC: bool = false;
                const SOL_NAME: &'static str = "uint256";
                #[inline]
                fn encode_body_len(&self) -> usize {
                    32
                }
                fn encode_body_to(&self, buf: &mut [u8]) {
                    U256::encode_body_to(&self.0, buf)
                }
            }
            impl StaticEncodedLen for Example {
                const ENCODED_SIZE: usize = 32;
            }
            impl SolDecode for Example {
                fn decode_at(input: &[u8], offset: usize) -> Result<Example, DecodeError> {
                    U256::decode_at(input, offset).map(|x| x.into())
                }
            }
            impl StaticDecode for Example {
                unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self {
                    unsafe { U256::decode_unchecked(input, offset).into() }
                }
            }
            #[derive(SolType, PartialEq, Eq, Debug)]
            pub struct Point {
                pub a: U256,
                pub b: U256,
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn enum_file_level_referenced_from_interface() {
        let file = quote! {
                enum B {First, Second}

                interface Ballot {
                    struct A { // Struct
                        B b;
                    }

                    function add(A memory b) external;
                }
        };
        let file = {
            let file = syn_solidity::parse2(file).unwrap();
            let tokens = expand_to_module(&file, true).unwrap().to_token_stream();
            prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
        };

        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod ballot {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface Ballot {
                struct A { B b; }
                function add(A memory b) external;
            }
            ```*/
                ///
                pub struct Ballot<
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
                > Ballot<Mutability, Inputs, Outputs, false> {
                    pub fn add(mut self, b: A) -> Ballot<NonPayable, (A), (), true> {
                        Ballot::<NonPayable, (A), (), true> {
                            address: self.address,
                            call_builder: CallBuilder::<NonPayable, (A), ()> {
                                payload: (b),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "add(", < A as ::pvm_contract_sdk::SolEncode > ::SOL_NAME,
                                            ")"
                                        ),
                                    )
                                },
                                witness: NonPayable::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl Ballot<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(address: Address) -> Ballot<Pure, (), (), false> {
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
                > Ballot<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> Ballot<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> Ballot<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > Ballot<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> Ballot<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(SolType, PartialEq, Eq, Debug)]
                pub struct A {
                    pub b: super::B,
                }
            }
            #[derive(PartialEq, Eq, Debug, ::pvm_contract_sdk::SolType)]
            #[repr(u8)]
            pub enum B {
                First,
                Second,
            }
        "#]]
        .assert_eq(&file);
    }

    #[test]
    fn enum_nested_as_param_and_return() {
        let file = quote! {
            #![abi_import(alloc = true)]
            pragma solidity ^0.8.0;
            interface VoteB {
                enum Color { Red, Green, Blue }
                function pick(Color c) external view returns (Color);
            }
        };
        let file = {
            let file = syn_solidity::parse2(file).unwrap();
            let tokens = expand_to_module(&file, true).unwrap().to_token_stream();
            prettyplease::unparse(&syn::File::parse.parse2(tokens).unwrap())
        };

        expect_test::expect![[r#"
            use pvm_contract_sdk::*;
            pub mod vote_b {
                use super::*;
                #[derive(Clone, Copy)]
                /// the code is derived from this interface
                /**```solidity
            interface VoteB {
                enum Color { Red, Green, Blue }
                function pick(Color c) external view returns (Color);
            }
            ```*/
                ///
                pub struct VoteB<
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
                > VoteB<Mutability, Inputs, Outputs, false> {
                    pub fn pick(mut self, c: Color) -> VoteB<View, (Color), (Color), true> {
                        VoteB::<View, (Color), (Color), true> {
                            address: self.address,
                            call_builder: CallBuilder::<View, (Color), (Color)> {
                                payload: (c),
                                selector: const {
                                    ::pvm_contract_sdk::const_selector(
                                        ::pvm_contract_sdk::const_format::concatcp!(
                                            "pick(", < Color as ::pvm_contract_sdk::SolEncode >
                                            ::SOL_NAME, ")"
                                        ),
                                    )
                                },
                                witness: View::default(),
                                call_limits: Default::default(),
                                allow_reentry: false,
                                _ret: core::marker::PhantomData,
                            },
                        }
                    }
                }
                impl VoteB<Pure, (), (), false> {
                    /// Create api for the contract from an address
                    pub fn from_address(address: Address) -> VoteB<Pure, (), (), false> {
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
                > VoteB<Mutability, Inputs, Outputs, true> {
                    /// Set call limits for the given call.
                    pub fn set_call_limits(mut self, limits: CallLimits) -> Self {
                        self.call_builder = self.call_builder.set_call_limits(limits);
                        self
                    }
                    /// Perform a delegated call to another contract.
                    ///
                    /// Always requires `&mut impl ContractContext` regardless of the
                    /// callee's declared mutability: the callee runs in caller's
                    /// storage context, so even a "view" callee can mutate state.
                    pub fn delegate_call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.delegate_call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a delegated call to another contract.
                    pub fn delegate_call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder
                            .delegate_call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> VoteB<View, Inputs, Outputs, true> {
                    /// Perform a call to a `view` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> VoteB<Pure, Inputs, Outputs, true> {
                    /// Perform a call to a `pure` callee.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(root.host()).max(512)
                        ];
                        self.call_builder.extract_output(root.host(), output_buf.as_mut_slice())
                    }
                }
                impl<
                    Inputs: SolEncode,
                    Outputs: SolDecode,
                > VoteB<NonPayable, Inputs, Outputs, true> {
                    /// Perform a call to a `nonpayable` callee. Caller must take
                    /// `&mut self` — `&self` (view) caller methods cannot construct
                    /// the `&mut impl ContractContext` argument.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                }
                impl<Inputs: SolEncode, Outputs: SolDecode> VoteB<Payable, Inputs, Outputs, true> {
                    /// Perform a call to a `payable` callee. Caller must take
                    /// `&mut self`.
                    pub fn call_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        input_buf: &mut [u8],
                        output_buf: &mut [u8],
                    ) -> Result<Outputs, CallError> {
                        self.call_builder.call(root, self.address, input_buf, output_buf)
                    }
                    /// Perform a call to another contract.
                    pub fn call<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                    ) -> Result<Outputs, CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 4 + self.call_builder.payload.encode_len()
                        ];
                        let host = root.host().clone();
                        self.call_builder.call_raw(root, self.address, input_buf.as_mut_slice())?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        self.call_builder.extract_output(&host, output_buf.as_mut_slice())
                    }
                    /// Instantiate another contract by it's code_hash. Always
                    /// requires `&mut impl ContractContext`: instantiation transfers
                    /// value, emits a deploy event, and bumps the caller's nonce.
                    pub fn instantiate_raw<R0: ContractContext>(
                        &self,
                        root: &mut R0,
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
                                root,
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
                    pub fn instantiate<R0: ContractContext>(
                        &self,
                        root: &mut R0,
                        code_hash: &[u8; 32],
                        value: u128,
                        limits: RefTimeAndProofSizeLimits,
                        salt: Option<&[u8; 32]>,
                    ) -> Result<(Address, Outputs), CallError> {
                        let mut input_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; 32 + self.call_builder.payload.encode_len()
                        ];
                        let mut address_buf = [0u8; 20];
                        let host = root.host().clone();
                        self.call_builder
                            .instantiate_raw(
                                root,
                                limits,
                                value,
                                code_hash,
                                salt,
                                &mut address_buf,
                                input_buf.as_mut_slice(),
                            )?;
                        let mut output_buf: alloc::vec::Vec<u8> = alloc::vec![
                            0; self.call_builder.output_size(& host).max(512)
                        ];
                        let output = self
                            .call_builder
                            .extract_output(&host, output_buf.as_mut_slice())?;
                        Ok((address_buf.into(), output))
                    }
                    /// Set the transfer `.value` of the call.
                    pub fn set_value(mut self, value: u128) -> Self {
                        self.call_builder = self.call_builder.set_value(value);
                        self
                    }
                }
                #[derive(PartialEq, Eq, Debug, ::pvm_contract_sdk::SolType)]
                #[repr(u8)]
                pub enum Color {
                    Red,
                    Green,
                    Blue,
                }
            }
        "#]]
        .assert_eq(&file);
    }
}
