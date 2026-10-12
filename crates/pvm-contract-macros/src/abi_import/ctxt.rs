use std::collections::{HashMap, HashSet};

use syn_solidity::{File, Item, ItemContract, ItemFunction, SolIdent, Spanned};

use crate::{
    signature::{CustomTypes, compute_selector},
    utils::{compute_function_signature, to_snake_case},
};

#[derive(Default)]
pub struct Ctxt {
    current_ns: Option<SolIdent>,
    // ns => name => set<signature>
    overloaded_functions: HashMap<Option<SolIdent>, HashMap<String, HashSet<String>>>,
    // ns => set[path]
    types: HashMap<Option<SolIdent>, HashSet<String>>,
    // definitions of the above, for expanding them in canonical signatures
    custom_types: CustomTypes,
}

/// Where a custom type path resolves, per Solidity scoping, relative to the
/// scope currently being expanded.
///
/// The single authority for path shape, visibility, and the Rust path prefix
/// the emission needs: `to_rust_type` matches on it exhaustively, so a new
/// scoping form (e.g. `import "..." as M;` making `M.IFoo.Point` legal) is a
/// new variant and a compile error at every consumer until handled.
pub enum Resolution {
    /// Declared in the scope currently being expanded (the current interface,
    /// or file level while expanding file-level items) — bare `Name`.
    Local,
    /// Unqualified, declared at file level, referenced from inside an
    /// interface module — `super::Name`.
    TopLevel,
    /// Qualified `Interface.Type`. `from_interface` distinguishes a reference
    /// from inside a sibling interface module (`super::#ns::Name`) from one in
    /// a file-level item spliced directly at the invocation site
    /// (`#ns::Name`).
    Qualified { ns: SolIdent, from_interface: bool },
}

impl Ctxt {
    /// Resolve a custom type path against this invocation's declarations.
    ///
    /// `abi_import!` input has no imports, so a path deeper than
    /// `Interface.Type` can never name anything declarable — solc rejects such
    /// a reference too. Splitting on first/last segments instead would
    /// silently resolve `A.X.Point` as `A.Point`.
    pub fn resolve(&self, path: &syn_solidity::SolPath) -> syn::Result<Resolution> {
        match path.len() {
            1 => {
                let name = path.first().to_string();
                // Current scope shadows file level. Within one invocation both
                // can't declare the same name (`CustomTypes::declare` rejects
                // duplicate simple names), so today the order is unobservable;
                // it becomes the solc-style precedence if that restriction is
                // ever lifted.
                if self.contains(&self.current_ns, &name) {
                    Ok(Resolution::Local)
                } else if self.current_ns.is_some() && self.contains(&None, &name) {
                    Ok(Resolution::TopLevel)
                } else {
                    Err(Self::unknown_type(path))
                }
            }
            2 => {
                if self.contains(&Some(path.first().clone()), &path.last().to_string()) {
                    Ok(Resolution::Qualified {
                        ns: path.first().clone(),
                        from_interface: self.current_ns.is_some(),
                    })
                } else {
                    Err(Self::unknown_type(path))
                }
            }
            _ => Err(syn::Error::new(
                path.span(),
                format!("qualified type paths must have the form `Interface.Type`: {path}"),
            )),
        }
    }

    fn contains(&self, ns: &Option<SolIdent>, name: &str) -> bool {
        self.types.get(ns).is_some_and(|set| set.contains(name))
    }

    fn unknown_type(path: &syn_solidity::SolPath) -> syn::Error {
        syn::Error::new(path.span(), format!("unknown type: {path}"))
    }

    pub fn set_ns(&mut self, ns: SolIdent) {
        self.current_ns = Some(ns);
    }

    pub fn with_ns<F: Fn(&mut Ctxt) -> R, R>(&mut self, ns: SolIdent, f: F) -> R {
        let past_ns = self.current_ns.clone();
        self.set_ns(ns);
        let res = f(self);
        self.current_ns = past_ns;
        res
    }

    /// The canonical signature of `item`, with any user-defined type expanded.
    pub fn function_signature(&self, item: &ItemFunction) -> String {
        compute_function_signature(item, &self.custom_types)
    }

    /// Whether `ty`'s canonical ABI form is dynamic, with user-defined types
    /// resolved through this invocation's declarations (enum → `uint8`,
    /// UDT → underlying, struct → its expanded fields).
    pub fn is_abi_dynamic(&self, ty: &syn_solidity::Type) -> bool {
        self.custom_types.is_abi_dynamic(ty)
    }

    pub fn function_name(&self, item: &ItemFunction) -> String {
        if self
            .overloaded_functions
            .get(&self.current_ns)
            .and_then(|f| f.get(&item.name().to_string()))
            .is_some_and(|x| x.len() > 1)
        {
            let name = to_snake_case(&item.name().as_string());

            format!(
                "{}_{}",
                name,
                const_hex::encode(compute_selector(&self.function_signature(item)))
            )
        } else {
            // `as_string()` strips the `r#` syn-solidity puts on keyword names;
            // the caller escapes the result back into a valid Rust identifier.
            to_snake_case(&item.name().as_string())
        }
    }

    pub fn visit_struct(&mut self, item: &syn_solidity::ItemStruct) {
        let ns = self.current_ns.clone();
        self.types
            .entry(ns)
            .or_default()
            .insert(item.name.to_string());
    }

    pub fn visit_error(&mut self, item: &syn_solidity::ItemError) {
        let ns = self.current_ns.clone();

        self.types
            .entry(ns)
            .or_default()
            .insert(item.name.to_string());
    }

    pub fn visit_udt(&mut self, item: &syn_solidity::ItemUdt) {
        let ns = self.current_ns.clone();

        self.types
            .entry(ns)
            .or_default()
            .insert(item.name.to_string());
    }

    pub fn visit_enum(&mut self, item: &syn_solidity::ItemEnum) {
        let ns = self.current_ns.clone();

        self.types
            .entry(ns)
            .or_default()
            .insert(item.name.to_string());
    }

    pub fn visit_file(&mut self, file: &File) -> Result<(), String> {
        // Signatures expand user-defined types, so every declaration in the
        // file has to be registered before any function is visited — a struct
        // may be declared after the function that takes it.
        self.custom_types = CustomTypes::from_file(file)?;
        file.items.iter().for_each(|item| match item {
            Item::Contract(contract) if contract.is_interface() => {
                self.with_ns(contract.name.clone(), |ctxt: &mut Ctxt| {
                    ctxt.visit_contract(contract);
                });
            }
            Item::Error(err) => self.visit_error(err),
            Item::Struct(struct_) => self.visit_struct(struct_),
            Item::Udt(udt) => self.visit_udt(udt),
            Item::Enum(enum_) => self.visit_enum(enum_),
            _ => (),
        });
        Ok(())
    }

    fn visit_contract(&mut self, contract: &ItemContract) {
        contract.body.iter().for_each(|item| match item {
            Item::Function(func) if func.name.is_some() => {
                self.visit_function(contract.name.clone(), func);
            }
            Item::Error(err) => self.visit_error(err),
            Item::Struct(struct_) => self.visit_struct(struct_),
            Item::Udt(udt) => self.visit_udt(udt),
            Item::Enum(enum_) => self.visit_enum(enum_),
            _ => (),
        })
    }

    fn visit_function(&mut self, ns: SolIdent, function: &ItemFunction) {
        let sig = self.function_signature(function);
        match self
            .overloaded_functions
            .entry(Some(ns))
            .or_default()
            .entry(function.name().to_string())
        {
            std::collections::hash_map::Entry::Occupied(occupied_entry) => {
                occupied_entry.into_mut().insert(sig);
            }
            std::collections::hash_map::Entry::Vacant(vacant_entry) => {
                let _ = vacant_entry.insert_entry(HashSet::from([sig]));
            }
        }
    }
}
