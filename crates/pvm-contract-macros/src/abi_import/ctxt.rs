use std::collections::{HashMap, HashSet};

use syn_solidity::{File, Item, ItemContract, ItemFunction, SolIdent};

use crate::{signature::compute_selector, solidity::to_snake_case};

#[derive(Default)]
pub struct Ctxt {
    current_ns: Option<SolIdent>,
    // ns => name => set<signature>
    overloaded_functions: HashMap<Option<SolIdent>, HashMap<String, HashSet<String>>>,
}

impl Ctxt {
    pub fn set_ns(&mut self, ns: SolIdent) {
        self.current_ns = Some(ns);
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
    pub fn visit_file(&mut self, file: &File) {
        file.items
            .iter()
            .filter_map(|item| match item {
                Item::Contract(contract) if contract.is_interface() => Some(contract),
                _ => None,
            })
            .for_each(|item| {
                self.visit_contract(item);
            });
    }
    fn visit_contract(&mut self, contract: &ItemContract) {
        contract
            .body
            .iter()
            .filter_map(|item| match item {
                Item::Function(func) => Some(func),
                _ => None,
            })
            .for_each(|item| {
                if item.name.is_some() {
                    self.visit_function(contract.name.clone(), item);
                }
            });
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

pub fn compute_function_signature(item: &ItemFunction) -> String {
    let mut name = format!("{}{}", item.name(), item.call_type());
    if name.rfind(",").is_some_and(|x| x == name.len() - 2) {
        name.remove(name.len() - 2);
    }
    name
}
