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
            // A size expression that is not a number literal (`uint256[N]` with
            // `N` a constant) makes `size()` return `None`, and `resolve` would
            // canonicalize it as the *dynamic* `uint256[]` — a silently wrong
            // selector. Constants are not evaluated, so reject instead.
            Type::Array(arr) if arr.size.is_some() && arr.size_lit().is_none() => {
                Err(pvm_contract_types::SOL_NON_LITERAL_ARRAY_SIZE.to_string())
            }
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
    fn non_literal_array_size_rejected() {
        // `uint256[N]` with `N` a named constant: `size()` is `None`, so the
        // type would canonicalize as the dynamic `uint256[]` and hash a
        // selector that differs from solc's folded `uint256[3]`.
        let src = "interface I { function f(uint256[N] xs) external; }";
        let file = syn_solidity::parse2(src.parse().unwrap()).unwrap();
        let err = CustomTypes::from_file(&file)
            .err()
            .expect("non-literal array size must be rejected");
        assert!(err.contains("number literal"), "{err}");
    }

    #[test]
    fn literal_array_size_accepted() {
        let t = types("interface I { function f(uint256[3] xs) external; }");
        assert_eq!(name_of(&t, "uint256[3]"), "uint256[3]");
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
