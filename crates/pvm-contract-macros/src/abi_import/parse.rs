use alloy_json_abi::ToSolConfig;
use std::path::{Path, PathBuf};
use syn::{
    Error, Expr, Ident, Lit, LitBool, LitStr, Result, Token,
    parse::{Parse, ParseStream},
};

pub(crate) fn parse_macro(input: ParseStream<'_>) -> Result<(syn_solidity::File, bool)> {
    let fork = input.fork();
    let (alloc_present, is_alloc) =
        if let Ok(syn::MetaNameValue { path, value, .. }) = fork.parse::<syn::MetaNameValue>() {
            (
                true,
                path.get_ident().is_some_and(|x| *x == "alloc")
                    && matches!(
                        value,
                        Expr::Lit(syn::ExprLit {
                            lit: Lit::Bool(LitBool { value: true, .. }),
                            ..
                        })
                    ),
            )
        } else {
            (false, false)
        };

    let fork = if alloc_present {
        let _ = fork.parse::<Token![,]>();
        let _ = input.parse::<syn::MetaNameValue>();
        let _ = input.parse::<Token![,]>();
        fork
    } else {
        input.fork()
    };
    // Include macro calls like `concat!(env!())`;
    let is_litstr_like = |fork: syn::parse::ParseStream<'_>| {
        fork.peek(LitStr) || (fork.peek(Ident) && fork.peek2(Token![!]))
    };

    if is_litstr_like(&fork)
        || (fork.peek(Ident) && fork.peek2(Token![,]) && {
            let _ = fork.parse::<Ident>();
            let _ = fork.parse::<Token![,]>();
            is_litstr_like(&fork)
        })
    {
        parse_json(input).map(|x| (x, is_alloc))
    } else if alloc_present {
        let content;

        syn::braced!(content in input);
        syn_solidity::File::parse(&content).map(|x| (x, is_alloc))
    } else {
        syn_solidity::File::parse(input).map(|x| (x, is_alloc))
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
