use std::collections::{HashMap, HashSet};

use syn_solidity::{File, Item, ItemContract, ItemFunction, SolIdent};

use crate::{
    signature::compute_selector,
    utils::{compute_function_signature, to_snake_case},
};

#[derive(Default)]
pub struct Ctxt {
    current_ns: Option<SolIdent>,
    // ns => name => set<signature>
    overloaded_functions: HashMap<Option<SolIdent>, HashMap<String, HashSet<String>>>,
    // ns => set[path]
    types: HashMap<Option<SolIdent>, HashSet<String>>,
    // enums:
    enums: HashMap<Option<SolIdent>, HashSet<String>>,
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

    pub fn is_enum(&self, path: syn_solidity::SolPath) -> bool {
        let (ns, name) = Self::parse_path(path);

        self.enums
            .get(&ns)
            .map(|map| map.contains(&name))
            .unwrap_or_default()
            || (if ns.is_none() {
                self.enums
                    .get(&self.current_ns)
                    .map(|map| map.contains(&name))
                    .unwrap_or_default()
            } else {
                false
            })
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
                const_hex::encode(compute_selector(&compute_function_signature(item)))
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

        self.enums
            .entry(ns)
            .or_default()
            .insert(item.name.to_string());
    }

    pub fn visit_file(&mut self, file: &File) {
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
        let sig = compute_function_signature(function);
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
