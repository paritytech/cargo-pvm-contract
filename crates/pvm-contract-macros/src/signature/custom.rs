use std::collections::HashMap;
use syn_solidity::{File, Item, Type};

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
    pub fn from_file(file: &File) -> Result<Self, String> {
        let mut this = Self::default();
        this.visit_items(&file.items)?;
        this.check_resolvable(&file.items)?;
        Ok(this)
    }

    /// Reject a custom type used in a function parameter/return or struct field
    /// that the file never declares: with no declaration there is nothing to
    /// expand, so `resolve` falls back to the bare name and hashes the selector
    /// from that rather than the type's canonical ABI form, silently producing
    /// the wrong selector. Imports aren't followed, so every referenced type
    /// must be declared here.
    fn check_resolvable(&self, items: &[Item]) -> Result<(), String> {
        for item in items {
            match item {
                Item::Struct(s) => {
                    for field in s.fields.iter() {
                        self.check_declared(&field.ty)?;
                    }
                }
                Item::Error(e) => {
                    for param in e.parameters.iter() {
                        self.check_declared(&param.ty)?;
                    }
                }
                Item::Function(f) => {
                    for ty in f.parameters.types() {
                        self.check_declared(ty)?;
                    }
                    if let Some(returns) = &f.returns {
                        for ty in returns.returns.types() {
                            self.check_declared(ty)?;
                        }
                    }
                }
                Item::Contract(c) => self.check_resolvable(&c.body)?,
                _ => (),
            }
        }
        Ok(())
    }

    /// Recurse through arrays/tuples and reject the first `Custom` type absent
    /// from `defs`.
    fn check_declared(&self, ty: &Type) -> Result<(), String> {
        match ty {
            Type::Array(arr) => self.check_declared(&arr.ty),
            Type::Tuple(tuple) => tuple.types.iter().try_for_each(|t| self.check_declared(t)),
            Type::Custom(path) => {
                let name = path.last().as_string();
                if self.defs.contains_key(&name) {
                    Ok(())
                } else {
                    Err(format!(
                        "undeclared type `{name}` in the `.sol` interface; declare it in this file \
                         (imports are not followed)"
                    ))
                }
            }
            _ => Ok(()),
        }
    }

    fn visit_items(&mut self, items: &[Item]) -> Result<(), String> {
        for item in items {
            match item {
                Item::Struct(x) => {
                    let fields = x.fields.iter().map(|f| f.ty.clone()).collect();
                    self.declare(x.name.as_string(), CustomDef::Struct(fields))?;
                }
                Item::Enum(x) => self.declare(x.name.as_string(), CustomDef::Enum)?,
                Item::Udt(x) => self.declare(x.name.as_string(), CustomDef::Udt(x.ty.clone()))?,
                Item::Contract(x) => self.visit_items(&x.body)?,
                _ => (),
            }
        }
        Ok(())
    }

    /// Rejects two declarations that share a simple name: types are keyed by
    /// that name, so a duplicate would otherwise silently resolve to whichever
    /// expanded last, corrupting the selector of every function that used the
    /// other.
    ///
    /// This uniqueness is also the invariant that keeps the two scoping models
    /// agreeing: `Ctxt::resolve` (abi_import) validates paths against a
    /// *namespaced* table, while this flat table answers `canonical_name` and
    /// `is_abi_dynamic` by `path.last()` alone. Lifting the restriction (e.g.
    /// to allow solc-style shadowing) requires namespacing this table first,
    /// or the gate and the selector may consult a different declaration than
    /// the one a qualified path resolves to.
    fn declare(&mut self, name: String, def: CustomDef) -> Result<(), String> {
        if self.defs.contains_key(&name) {
            return Err(format!(
                "two Solidity user-defined types are both named `{name}`; \
                 rename one in the interface to avoid the collision"
            ));
        }
        self.defs.insert(name, def);
        Ok(())
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

    /// Whether `ty`'s canonical ABI form is dynamic, resolving user-defined
    /// types: an enum is `uint8` (static), a UDT is its underlying type, and a
    /// struct is dynamic iff any expanded field is. syn-solidity's own
    /// `Type::is_abi_dynamic` hardcodes `Custom(_) => true` — it has no
    /// declaration table to consult.
    pub fn is_abi_dynamic(&self, ty: &Type) -> bool {
        self.is_dynamic(ty, &mut Vec::new())
    }

    /// `active` mirrors `resolve`'s cycle guard. For solc-legal input a struct
    /// may reference itself only through a dynamic array, so the enclosing
    /// `Array` arm decides before the stack is consulted; for an illegal
    /// direct cycle (`A { B b; } B { A a; }`) the on-stack hit terminates the
    /// walk and `true` is the conservative answer.
    fn is_dynamic(&self, ty: &Type, active: &mut Vec<String>) -> bool {
        match ty {
            Type::String(_) | Type::Bytes(_) => true,
            Type::Array(arr) => arr.size().is_none() || self.is_dynamic(&arr.ty, active),
            Type::Tuple(tuple) => tuple.types.iter().any(|t| self.is_dynamic(t, active)),
            Type::Custom(path) => {
                let name = path.last().as_string();
                if active.contains(&name) {
                    return true;
                }
                match self.defs.get(&name) {
                    Some(CustomDef::Enum) => false,
                    Some(CustomDef::Udt(underlying)) => self.is_dynamic(underlying, active),
                    Some(CustomDef::Struct(fields)) => {
                        active.push(name);
                        let out = fields.iter().any(|f| self.is_dynamic(f, active));
                        active.pop();
                        out
                    }
                    // Undeclared: `check_resolvable` reports struct fields and
                    // function params/returns (not `error` params); stay
                    // conservative here either way.
                    None => true,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(src: &str) -> CustomTypes {
        CustomTypes::from_file(&syn_solidity::parse2(src.parse().unwrap()).unwrap()).unwrap()
    }

    fn name_of(types: &CustomTypes, ty: &str) -> String {
        types.canonical_name(&syn::parse_str::<Type>(ty).unwrap())
    }

    #[test]
    fn dynamism_resolves_user_defined_types() {
        let t = types(
            "enum Status { Open, Closed }
             type Count is uint64;
             struct Point { uint64 x; uint64 y; }
             struct Line { Point a; Point b; }
             struct Named { string name; }
             struct Card { Named n; }
             struct Tree { Tree[] children; }",
        );
        let dynamic = |ty: &str| t.is_abi_dynamic(&syn::parse_str::<Type>(ty).unwrap());
        assert!(!dynamic("Status"));
        assert!(!dynamic("Count"));
        assert!(!dynamic("Point"));
        assert!(!dynamic("Point[2]"));
        assert!(dynamic("Point[]"));
        assert!(dynamic("Named"));
        // Nesting alone doesn't make a struct dynamic — its expanded leaves do.
        assert!(!dynamic("Line"));
        assert!(dynamic("Card"));
        // A self-referential struct terminates and is dynamic.
        assert!(dynamic("Tree"));
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

    #[test]
    fn same_named_types_collide() {
        let src = "interface A { struct Point { uint64 x; } }
                   interface B { struct Point { uint64 x; uint64 y; } }";
        let file = syn_solidity::parse2(src.parse().unwrap()).unwrap();
        let err = CustomTypes::from_file(&file)
            .err()
            .expect("expected a collision error");
        assert!(err.contains("both named `Point`"), "{err}");
    }

    #[test]
    fn qualified_same_named_types_are_rejected() {
        let src = r#"
            interface IFoo { struct Point { uint64 x; uint64 y; } }
            interface Irrelevant { struct Point { bool exists; } }
            interface Relevant { function add(IFoo.Point a, IFoo.Point b) external; }
        "#;
        let file = syn_solidity::parse2(src.parse().unwrap()).unwrap();
        let err = CustomTypes::from_file(&file)
            .err()
            .expect("two `Point` types must be rejected");
        assert!(err.contains("both named `Point`"), "{err}");
    }

    #[test]
    fn undeclared_custom_type_is_rejected() {
        let src = "interface I { function f(Missing m) external; }";
        let file = syn_solidity::parse2(src.parse().unwrap()).unwrap();
        let err = CustomTypes::from_file(&file)
            .err()
            .expect("undeclared type must be rejected");
        assert!(err.contains("undeclared type `Missing`"), "{err}");
    }

    #[test]
    fn same_named_udt_aliases_collide() {
        // Two `type X is ...` aliases sharing a simple name: keyed by simple
        // name like structs, so a duplicate is rejected rather than silently
        // resolving to one underlying type.
        let src = "interface A { type Id is uint64; }
                   interface B { type Id is address; }";
        let file = syn_solidity::parse2(src.parse().unwrap()).unwrap();
        let err = CustomTypes::from_file(&file)
            .err()
            .expect("two `Id` aliases must be rejected");
        assert!(err.contains("both named `Id`"), "{err}");
    }

    #[test]
    fn mutually_recursive_structs_terminate() {
        // solc rejects these as infinitely sized, but syn_solidity parses them,
        // so the resolver must not recurse forever: a name already being
        // expanded falls back to its bare form.
        let t = types(
            "struct A { B b; uint64 v; }
             struct B { A a; }",
        );
        assert_eq!(name_of(&t, "A"), "((A),uint64)");
    }
}
