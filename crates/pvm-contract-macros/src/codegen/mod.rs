pub mod abi_import;
mod allocator;
mod contract;
mod decode;
mod derive_sol_abi;
mod dispatch;
mod encode;
mod method;
mod storage;

pub use abi_import::{expand_abi_import, AbiImportArgs};
pub use contract::{expand_contract, ContractArgs};
pub use derive_sol_abi::expand_derive_sol_abi;
pub use method::{expand_constructor, expand_fallback, expand_method, MethodArgs};
pub use storage::expand_storage;

/// If `ty` is `Option<Inner>`, return `Some(Inner)`. Otherwise `None`.
fn unwrap_option_inner(ty: &syn::Type) -> Option<&syn::Type> {
    if let syn::Type::Path(type_path) = ty {
        let segment = type_path.path.segments.last()?;
        if segment.ident == "Option" {
            if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                if args.args.len() == 1 {
                    if let syn::GenericArgument::Type(inner) = &args.args[0] {
                        return Some(inner);
                    }
                }
            }
        }
    }
    None
}
