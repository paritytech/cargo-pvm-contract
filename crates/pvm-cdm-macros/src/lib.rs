//! Proc-macro implementation for the `pvm-cdm` crate.
//!
//! The public entry point is [`reference!`], which attaches CDM-aware
//! constructors (`cdm_lookup`, `cdm_from_env`) as inherent methods on an
//! `abi_import!`-generated contract type. See the `pvm-cdm` crate docs for
//! usage.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{LitStr, Path, Token, parse::Parse, parse::ParseStream, parse_macro_input};

/// Compile-time selector for `getAddress(string)`, the one registry ABI
/// method this macro targets. Baked at build time of this crate.
const GET_ADDRESS_SELECTOR: [u8; 4] = {
    let hash = keccak_const::Keccak256::new()
        .update(b"getAddress(string)")
        .finalize();
    [hash[0], hash[1], hash[2], hash[3]]
};

struct ReferenceInput {
    contract_ty: Path,
    cdm_name: String,
}

impl Parse for ReferenceInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let contract_ty: Path = input.parse()?;
        input.parse::<Token![,]>()?;
        let name_lit: LitStr = input.parse()?;
        if !input.is_empty() {
            input.parse::<Option<Token![,]>>()?;
            if !input.is_empty() {
                return Err(input.error("unexpected tokens after CDM package name"));
            }
        }
        Ok(Self {
            contract_ty,
            cdm_name: name_lit.value(),
        })
    }
}

/// Attach `cdm_lookup()` and `cdm_from_env()` to an imported contract type.
///
/// ```ignore
/// pvm_cdm::reference!(foo::Foo, "@example/foo");
/// ```
///
/// Expands to an inherent `impl` block on `foo::Foo<Pure, (), (), false>` with:
///
/// - `pub fn cdm_lookup() -> Self` — runtime registry lookup via
///   `ContractRegistry.getAddress(string)`, with the registry address baked
///   from `CONTRACTS_REGISTRY_ADDR`.
/// - `pub fn cdm_from_env() -> Self` — address baked from `CDM_REGISTRY`
///   (`name=hex;name=hex;...`) at compile time.
///
/// Both env vars are read with `option_env!`: if only one is set, the other
/// resolver's address falls back to the zero sentinel and panics at runtime
/// with a descriptive message when invoked.
#[proc_macro]
pub fn reference(input: TokenStream) -> TokenStream {
    let ReferenceInput {
        contract_ty,
        cdm_name,
    } = parse_macro_input!(input as ReferenceInput);

    let [s0, s1, s2, s3] = GET_ADDRESS_SELECTOR;

    let expanded: TokenStream2 = quote! {
        impl #contract_ty<
            ::pvm_contract_sdk::Pure,
            (),
            (),
            false,
        > {
            /// Resolve this contract's address via a runtime call to the CDM registry.
            ///
            /// Panics if `CONTRACTS_REGISTRY_ADDR` was unset at build time, if the
            /// registry call fails, or if the package isn't registered.
            pub fn cdm_lookup() -> Self {
                extern crate alloc;

                const fn __hex_nibble(c: u8) -> u8 {
                    match c {
                        b'0'..=b'9' => c - b'0',
                        b'a'..=b'f' => c - b'a' + 10,
                        b'A'..=b'F' => c - b'A' + 10,
                        _ => panic!("CDM address contains an invalid hex character"),
                    }
                }

                const __REGISTRY_ADDR: [u8; 20] = match option_env!("CONTRACTS_REGISTRY_ADDR") {
                    Some(s) => {
                        let b = s.as_bytes();
                        let off = if b.len() > 1 && b[0] == b'0' && (b[1] == b'x' || b[1] == b'X') {
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
                            r[i] = __hex_nibble(b[off + i * 2]) << 4
                                | __hex_nibble(b[off + i * 2 + 1]);
                            i += 1;
                        }
                        r
                    }
                    None => [0u8; 20],
                };

                if __REGISTRY_ADDR == [0u8; 20] {
                    panic!(concat!(
                        "cdm_lookup(): CONTRACTS_REGISTRY_ADDR env var must be set ",
                        "at build time to a 40-hex-char registry address"
                    ));
                }

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

                let result = <::pvm_contract_sdk::PolkaVmHost as ::pvm_contract_sdk::HostApi>::call_evm(
                    ::pvm_contract_sdk::CallFlags::ALLOW_REENTRY,
                    &__REGISTRY_ADDR,
                    u64::MAX,
                    &[0u8; 32],
                    &calldata,
                    Some(&mut output_ref),
                );

                match result {
                    Ok(()) => {
                        let is_some = output_buf[31] != 0;
                        if !is_some {
                            panic!("cdm_lookup: contract not found in registry");
                        }
                        let mut addr = [0u8; 20];
                        addr.copy_from_slice(&output_buf[44..64]);
                        Self::from_address(addr.into())
                    }
                    Err(_) => panic!("cdm_lookup: registry call failed"),
                }
            }

            /// Resolve this contract's address from the compile-time `CDM_REGISTRY`
            /// mapping (`name=hex;name=hex;...`).
            ///
            /// Panics at runtime if `CDM_REGISTRY` was unset or missing an entry
            /// for this package's name.
            pub fn cdm_from_env() -> Self {
                const fn __hex_nibble(c: u8) -> u8 {
                    match c {
                        b'0'..=b'9' => c - b'0',
                        b'a'..=b'f' => c - b'a' + 10,
                        b'A'..=b'F' => c - b'A' + 10,
                        _ => panic!("CDM address contains an invalid hex character"),
                    }
                }

                const __FROM_ENV_ADDR: [u8; 20] = {
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
                                    if start + 1 < bytes.len()
                                        && bytes[start] == b'0'
                                        && (bytes[start + 1] == b'x' || bytes[start + 1] == b'X')
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
                                        r[i] = __hex_nibble(bytes[start + i * 2]) << 4
                                            | __hex_nibble(bytes[start + i * 2 + 1]);
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

                if __FROM_ENV_ADDR == [0u8; 20] {
                    panic!(concat!(
                        "cdm_from_env(): CDM_REGISTRY env var must be set at build time ",
                        "and contain an entry for `", #cdm_name, "=<hexaddress>`"
                    ));
                }
                Self::from_address(__FROM_ENV_ADDR.into())
            }
        }
    };

    expanded.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn parse(src: &str) -> syn::Result<ReferenceInput> {
        syn::parse_str(src)
    }

    #[test]
    fn parses_simple_path_and_name() {
        let input = parse(r#"Foo, "@ns/foo""#).unwrap();
        assert_eq!(input.contract_ty.to_token_stream().to_string(), "Foo");
        assert_eq!(input.cdm_name, "@ns/foo");
    }

    #[test]
    fn parses_nested_module_path() {
        let input = parse(r#"foo::Foo, "@ns/foo""#).unwrap();
        assert_eq!(
            input.contract_ty.to_token_stream().to_string(),
            "foo :: Foo"
        );
    }

    #[test]
    fn parses_absolute_path() {
        let input = parse(r#"::my_crate::foo::Foo, "@ns/foo""#).unwrap();
        assert_eq!(
            input.contract_ty.to_token_stream().to_string(),
            ":: my_crate :: foo :: Foo"
        );
    }

    #[test]
    fn accepts_trailing_comma() {
        parse(r#"Foo, "@ns/foo","#).unwrap();
    }

    #[test]
    fn rejects_missing_name() {
        assert!(parse("Foo").is_err());
    }

    #[test]
    fn rejects_missing_type() {
        assert!(parse(r#""@ns/foo""#).is_err());
    }

    #[test]
    fn rejects_extra_args() {
        assert!(parse(r#"Foo, "@ns/foo", extra"#).is_err());
    }

    #[test]
    fn get_address_selector_matches_abi_spec() {
        // keccak256("getAddress(string)")[..4]
        assert_eq!(GET_ADDRESS_SELECTOR, [0xbf, 0x40, 0xfa, 0xc1]);
    }
}
