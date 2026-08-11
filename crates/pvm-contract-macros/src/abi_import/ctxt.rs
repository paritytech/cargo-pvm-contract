use std::collections::{HashMap, HashSet};

use syn_solidity::{File, Item, ItemContract, ItemFunction, SolIdent};

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

impl Ctxt {
    fn parse_path(path: syn_solidity::SolPath) -> (Option<SolIdent>, String) {
        if path.len() == 1 {
            (None, path.first().to_string())
        } else {
            (Some(path.first().clone()), path.last().to_string())
        }
    }

    pub fn resolve_type(&self, path: syn_solidity::SolPath) -> bool {
        let (ns, name) = Self::parse_path(path.clone());
        self.types
            .get(&ns)
            .map(|map| map.contains(&name))
            .unwrap_or_default()
            || (if ns.is_none() {
                self.types
                    .get(&self.current_ns)
                    .map(|map| map.contains(&name))
                    .unwrap_or_default()
            } else {
                false
            })
    }

    pub fn is_in_toplevel(&self, path: syn_solidity::SolPath) -> bool {
        let (ns, name) = Self::parse_path(path.clone());
        if ns.is_none() {
            self.types
                .get(&None)
                .map(|map| map.contains(&name))
                .unwrap_or_default()
        } else {
            false
        }
    }

    pub fn is_in_current_scope(&self, path: syn_solidity::SolPath) -> bool {
        let (ns, name) = Self::parse_path(path.clone());
        if ns.is_none() {
            self.types
                .get(&self.current_ns)
                .map(|map| map.contains(&name))
                .unwrap_or_default()
        } else {
            false
        }
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

    pub fn function_name(&self, item: &ItemFunction) -> String {
        if self
            .overloaded_functions
            .get(&self.current_ns)
            .and_then(|f| f.get(&item.name().to_string()))
            .is_some_and(|x| x.len() > 1)
        {
            let name = to_snake_case(&item.name().to_string());

            format!(
                "{}_{}",
                name,
                const_hex::encode(compute_selector(&self.function_signature(item)))
            )
        } else {
            to_snake_case(&item.name().to_string())
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
