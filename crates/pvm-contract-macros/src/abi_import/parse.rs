use alloy_json_abi::ToSolConfig;
use std::path::{Path, PathBuf};
use syn::{
    Attribute, Error, Ident, LitBool, LitStr, Result, Token,
    parse::{Parse, ParseStream},
};

pub(crate) fn parse_macro(input: ParseStream<'_>) -> Result<(syn_solidity::File, bool)> {
    let attrs = Attribute::parse_inner(input)?;
    let fork = input.fork();

    // Include macro calls like `concat!(env!())`;
    let is_litstr_like = |fork: syn::parse::ParseStream<'_>| {
        fork.peek(LitStr) || (fork.peek(Ident) && fork.peek2(Token![!]))
    };

    let (abi_attrs, rest) = AbiAttrs::parse(&attrs)?;
    if !rest.is_empty() {
        return Err(syn::Error::new_spanned(
            rest.first().unwrap(),
            "only `#[abi_import]` attributes are allowed here",
        ));
    }
    if is_litstr_like(&fork)
        || (fork.peek(Ident) && fork.peek2(Token![,]) && {
            let _ = fork.parse::<Ident>();
            let _ = fork.parse::<Token![,]>();
            is_litstr_like(&fork)
        })
    {
        parse_json(input).map(|x| (x, abi_attrs.alloc.unwrap_or_default()))
    } else {
        syn_solidity::File::parse(input).map(|x| (x, abi_attrs.alloc.unwrap_or_default()))
    }
}

fn parse_json(input: ParseStream<'_>) -> Result<syn_solidity::File> {
    let name = input.parse::<Option<Ident>>()?;
    if name.is_some() {
        input.parse::<Token![,]>()?;
    }
    let span = input.span();
    let macro_string = input.parse::<macro_string::MacroString>()?;
    let mut value = macro_string.eval()?;

    let _ = input.parse::<Option<Token![,]>>()?;
    if !input.is_empty() {
        let msg = "unexpected token, expected end of input";
        return Err(Error::new(input.span(), msg));
    }

    let mut p = PathBuf::from(value);
    if p.is_relative() {
        let dir = std::env::var_os("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| Error::new(span, "failed to get manifest dir"))?;
        p = dir.join(p);
    }
    p = std::fs::canonicalize(&p)
        .map_err(|e| Error::new(span, format!("failed to canonicalize path {p:?}: {e}")))?;
    value = std::fs::read_to_string(&p)
        .map_err(|e| Error::new(span, format!("failed to read file {p:?}: {e}")))?;
    let path = p;

    let s = value.trim();
    if s.is_empty() {
        let msg = { "file path is empty" };
        return Err(Error::new(span, msg));
    }
    let span = input.span();
    load_json_abi(
        name.map_or("contract".to_owned(), |x| x.to_string()),
        span,
        &path,
    )
}

pub(crate) fn load_json_abi(
    name: String,
    token_span: proc_macro2::Span,
    path: &Path,
) -> Result<syn_solidity::File> {
    let file = std::fs::read_to_string(path)
        .map_err(|err| syn::Error::new(token_span, err.to_string()))?;

    let parsed: alloy_json_abi::JsonAbi =
        serde_json::from_str(&file).map_err(|err| syn::Error::new(token_span, err.to_string()))?;
    let config = ToSolConfig::new()
        .print_constructors(true)
        .for_sol_macro(true);

    let unparsed = &parsed.to_sol(&name, Some(config));
    let tts = syn::parse_str::<proc_macro2::TokenStream>(unparsed)
        .map_err(|e| syn::Error::new(token_span, &e))?;

    syn_solidity::parse2(quote::quote! {
        #tts
    })
}

const DUPLICATE_ERROR: &str = "duplicate attribute";
const UNKNOWN_ERROR: &str = "unknown `abi_import` attribute";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AbiAttrs {
    /// `#[abi_import(alloc)]`
    pub alloc: Option<bool>,
}

impl AbiAttrs {
    /// Parse the `#[abi_import(...)]` attributes from a list of attributes.
    pub fn parse(attrs: &[Attribute]) -> Result<(Self, Vec<Attribute>)> {
        let mut this = Self::default();
        let mut others = Vec::with_capacity(attrs.len());
        for attr in attrs {
            if !attr.path().is_ident("abi_import") {
                others.push(attr.clone());
                continue;
            }

            attr.meta.require_list()?.parse_nested_meta(|meta| {
                let path = meta
                    .path
                    .get_ident()
                    .ok_or_else(|| meta.error("expected ident"))?;
                let s = path.to_string();

                macro_rules! match_ {
                    ($($l:ident => $e:expr),* $(,)?) => {
                        match s.as_str() {
                            $(
                                stringify!($l) => if this.$l.is_some() {
                                    return Err(meta.error(DUPLICATE_ERROR))
                                } else {
                                    this.$l = Some($e);
                                },
                            )*
                            _ => return Err(meta.error(UNKNOWN_ERROR)),
                        }
                    };
                }

                // `path` => true, `path = <bool>` => <bool>
                let bool = || {
                    if let Ok(input) = meta.value() {
                        input.parse::<LitBool>().map(|lit| lit.value)
                    } else {
                        Ok(true)
                    }
                };

                match_! {
                    alloc => bool()?,
                };
                Ok(())
            })?;
        }
        Ok((this, others))
    }
}
