use std::collections::HashMap;
use syn_solidity::{File, Item, ItemContract, ItemEnum, ItemStruct, ItemUdt, Type};

/// A user-defined Solidity type, reduced to what a canonical signature needs.
enum CustomDef {
    /// Field types, in declaration order.
    Struct(Vec<Type>),
    Enum,
    /// The underlying value type of a `type X is T;` declaration.
    Udt(Type),
}

/// The user-defined types declared in a `.sol` source, used to expand a custom
/// type into its canonical ABI name when computing a selector: a struct becomes
/// the tuple of its field types, an `enum` becomes `uint8`, and a user-defined
/// value type becomes its underlying type.
///
/// Keyed by simple name — `IFoo.Point` and `Point` both resolve to `Point` —
/// matching the ABI generator in `cargo-pvm-contract-builder`.
#[derive(Default)]
pub struct CustomTypes {
    defs: HashMap<String, CustomDef>,
}

impl CustomTypes {
    pub fn from_file(file: &File) -> Self {
        let mut this = Self::default();
        this.visit_items(&file.items);
        this
    }

    fn visit_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Struct(x) => self.visit_struct(x),
                Item::Enum(x) => self.visit_enum(x),
                Item::Udt(x) => self.visit_udt(x),
                Item::Contract(x) => self.visit_contract(x),
                _ => (),
            }
        }
    }

    fn visit_contract(&mut self, contract: &ItemContract) {
        self.visit_items(&contract.body);
    }

    fn visit_struct(&mut self, item: &ItemStruct) {
        let fields = item.fields.iter().map(|f| f.ty.clone()).collect();
        self.defs
            .insert(item.name.as_string(), CustomDef::Struct(fields));
    }

    fn visit_enum(&mut self, item: &ItemEnum) {
        self.defs.insert(item.name.as_string(), CustomDef::Enum);
    }

    fn visit_udt(&mut self, item: &ItemUdt) {
        self.defs
            .insert(item.name.as_string(), CustomDef::Udt(item.ty.clone()));
    }

    /// The canonical ABI name of `ty`, as it appears in a function signature.
    pub fn canonical_name(&self, ty: &Type) -> String {
        self.resolve(ty, &mut Vec::new())
    }

    /// `active` is the stack of struct names currently being expanded: a struct
    /// may legally reference itself through a dynamic array
    /// (`struct S { S[] children; }`), so a name already on the stack falls back
    /// to its bare name — the same fallback used for unresolved custom types.
    fn resolve(&self, ty: &Type, active: &mut Vec<String>) -> String {
        match ty {
            Type::Address(_, _) => "address".to_string(),
            Type::Bool(_) => "bool".to_string(),
            Type::String(_) => "string".to_string(),
            Type::Bytes(_) => "bytes".to_string(),
            Type::FixedBytes(_, size) => format!("bytes{size}"),
            // A declaration may spell these `int`/`uint`; the canonical name is
            // always explicit about the width.
            Type::Int(_, size) => format!("int{}", size.map(|s| s.get()).unwrap_or(256)),
            Type::Uint(_, size) => format!("uint{}", size.map(|s| s.get()).unwrap_or(256)),
            // Not valid ABI parameter types; reported elsewhere.
            Type::Mapping(_) | Type::Function(_) => ty.to_string(),
            Type::Array(arr) => {
                let inner = self.resolve(&arr.ty, active);
                match arr.size() {
                    Some(n) => format!("{inner}[{n}]"),
                    None => format!("{inner}[]"),
                }
            }
            Type::Tuple(tuple) => self.tuple_name(tuple.types.iter(), active),
            Type::Custom(path) => {
                let name = path.last().as_string();
                if active.contains(&name) {
                    return name;
                }
                match self.defs.get(&name) {
                    Some(CustomDef::Enum) => "uint8".to_string(),
                    Some(CustomDef::Udt(underlying)) => self.resolve(underlying, active),
                    Some(CustomDef::Struct(fields)) => {
                        active.push(name);
                        let out = self.tuple_name(fields.iter(), active);
                        active.pop();
                        out
                    }
                    None => name,
                }
            }
        }
    }

    fn tuple_name<'a>(
        &self,
        types: impl Iterator<Item = &'a Type>,
        active: &mut Vec<String>,
    ) -> String {
        let inner: Vec<String> = types.map(|t| self.resolve(t, active)).collect();
        format!("({})", inner.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(src: &str) -> CustomTypes {
        CustomTypes::from_file(&syn_solidity::parse2(src.parse().unwrap()).unwrap())
    }

    fn name_of(types: &CustomTypes, ty: &str) -> String {
        types.canonical_name(&syn::parse_str::<Type>(ty).unwrap())
    }

    #[test]
    fn primitives_keep_their_solidity_name() {
        let t = CustomTypes::default();
        assert_eq!(name_of(&t, "uint256"), "uint256");
        assert_eq!(name_of(&t, "uint"), "uint256");
        assert_eq!(name_of(&t, "address"), "address");
        assert_eq!(name_of(&t, "bytes32[3]"), "bytes32[3]");
        assert_eq!(name_of(&t, "(bool,string)"), "(bool,string)");
    }

    #[test]
    fn struct_expands_to_its_field_tuple() {
        let t = types("struct Point { uint64 x; uint64 y; }");
        assert_eq!(name_of(&t, "Point"), "(uint64,uint64)");
        assert_eq!(name_of(&t, "Point[]"), "(uint64,uint64)[]");
        assert_eq!(name_of(&t, "Point[2]"), "(uint64,uint64)[2]");
    }

    #[test]
    fn nested_struct_expands_recursively() {
        let t = types(
            "struct Point { uint64 x; uint64 y; }
             struct Line { Point a; Point b; }",
        );
        assert_eq!(
            name_of(&t, "Line"),
            "((uint64,uint64),(uint64,uint64))".to_string()
        );
    }

    #[test]
    fn struct_declared_inside_an_interface_is_visible() {
        let t = types("interface IFoo { struct Point { uint64 x; uint64 y; } }");
        assert_eq!(name_of(&t, "Point"), "(uint64,uint64)");
        assert_eq!(name_of(&t, "IFoo.Point"), "(uint64,uint64)");
    }

    #[test]
    fn enum_is_uint8_and_udt_is_its_underlying_type() {
        let t = types(
            "enum Status { Open, Closed }
             type Count is uint64;",
        );
        assert_eq!(name_of(&t, "Status"), "uint8");
        assert_eq!(name_of(&t, "Count"), "uint64");
    }

    #[test]
    fn unknown_custom_type_keeps_its_bare_name() {
        let t = CustomTypes::default();
        assert_eq!(name_of(&t, "Mystery"), "Mystery");
    }

    #[test]
    fn self_referential_struct_terminates() {
        let t = types("struct S { S[] children; uint256 v; }");
        assert_eq!(name_of(&t, "S"), "(S[],uint256)");
    }
}
