//! The `.sol`-derived fixture ABIs against the real solc.
//!
//! Every other check in the suite compares our emitter with viem, and viem
//! trusts whatever ABI it is handed. solc is the one oracle that derives an
//! ABI from the same Solidity source independently, so this is the check that
//! would catch our parser and viem agreeing on something solc would never have
//! produced. Needs `solc` on PATH; gated like the other solc differentials.
//!
//! Normalisation, both sides to a sorted set of canonical JSON items:
//! - solc annotates every parameter with `internalType`; the encoding does not
//!   depend on it and our emitter does not produce it, so it is stripped.
//! - our emitter appends the SDK's five framework errors to every ABI; solc
//!   has never heard of them, so they are stripped from our side.
//! - solc emits one ABI per contract in the file, inherited items included; our
//!   parser flattens every interface in the file into one ABI. The oracle is
//!   therefore the union of solc's per-contract ABIs.

#![cfg(feature = "solc-tests")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `.sol` files whose fixture is *expected* to disagree with solc, with the
/// reason. Each is asserted to still disagree, so a fix cannot go unnoticed.
const KNOWN_DIVERGENT: &[(&str, &str)] = &[(
    "crates/pvm-viem-roundtrip/sol/SolReferenceTypes.sol",
    "contract-typed parameters are emitted as `IToken` instead of `address`, and solc \
     refuses to compile the self-referential `Node` in an external signature at all \
     (known gap: known-gaps/emitter.test.ts)",
)];

/// `.sol` files solc cannot compile at all, with the reason. Our parser is
/// more permissive than solc in these spots, which is itself worth knowing.
const NOT_SOLIDITY: &[(&str, &str)] = &[
    (
        "examples/test-contracts/CompositeTypes.sol",
        "declares an inline tuple parameter `(uint256,bool)`, which is not valid Solidity \
         syntax; our parser accepts it as an extension",
    ),
    (
        "crates/pvm-viem-roundtrip/sol/SolTypeSurface.sol",
        "same inline-tuple extension (`inlineTuple`)",
    ),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").exists() && dir.join("crates").is_dir())
        .map(Path::to_path_buf)
        .expect("workspace root")
}

/// Every `.sol` file a fixture is derived from: the `.sol` interfaces of the
/// test contracts the generator lists, plus this crate's standalone ones.
fn fixture_sol_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root.join("examples/test-contracts")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "sol") {
            files.push(path);
        }
    }
    for entry in std::fs::read_dir(root.join("crates/pvm-viem-roundtrip/sol")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "sol") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn strip_internal_type(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("internalType");
            for v in map.values_mut() {
                strip_internal_type(v);
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(strip_internal_type),
        _ => {}
    }
}

/// Canonical, order-independent form of an ABI item list.
fn canonical(items: &[serde_json::Value]) -> BTreeSet<String> {
    items
        .iter()
        .map(|item| {
            let mut item = item.clone();
            strip_internal_type(&mut item);
            // serde_json maps serialize with sorted keys, so this is canonical.
            serde_json::to_string(&item).unwrap()
        })
        .collect()
}

/// solc's ABI for every contract in the file, or the compiler's diagnostics.
fn solc_abi_union(sol: &Path) -> Result<Vec<serde_json::Value>, String> {
    let out = Command::new("solc")
        .args(["--combined-json", "abi"])
        .arg(sol)
        .output()
        .expect("spawn solc — is it installed and on PATH?");
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut union = Vec::new();
    for (_name, contract) in parsed["contracts"].as_object().unwrap() {
        // Older solc versions nest the ABI as a JSON string.
        let abi = match &contract["abi"] {
            serde_json::Value::String(s) => serde_json::from_str(s).unwrap(),
            other => other.clone(),
        };
        union.extend(abi.as_array().unwrap().iter().cloned());
    }
    Ok(union)
}

fn ours(sol: &Path) -> Vec<serde_json::Value> {
    let json = cargo_pvm_contract_builder::render_abi_from_sol(sol)
        .unwrap()
        .unwrap_or_else(|| panic!("{} rendered no ABI", sol.display()));
    let items: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    let framework: Vec<String> = pvm_contract_types::framework_errors::NAMES
        .iter()
        .map(|n| n.to_string())
        .collect();
    items
        .into_iter()
        .filter(|item| {
            !(item["type"] == "error"
                && item["inputs"].as_array().is_some_and(|i| i.is_empty())
                && item["name"]
                    .as_str()
                    .is_some_and(|n| framework.iter().any(|f| f == n)))
        })
        .collect()
}

#[test]
fn every_sol_fixture_abi_matches_solc() {
    let root = repo_root();
    let files = fixture_sol_files(&root);
    assert!(files.len() >= 20, "found only {} .sol files", files.len());

    let mut divergent_seen = BTreeSet::new();
    let mut not_solidity_seen = BTreeSet::new();
    let mut mismatches = Vec::new();

    for sol in &files {
        let rel = sol
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");

        if NOT_SOLIDITY.iter().any(|(f, _)| *f == rel) {
            // Must still fail to compile, otherwise the entry is stale.
            let status = Command::new("solc").arg("--abi").arg(sol).output().unwrap();
            assert!(
                !status.status.success(),
                "{rel} now compiles with solc; remove it from NOT_SOLIDITY and let the oracle cover it"
            );
            not_solidity_seen.insert(rel);
            continue;
        }

        let actual = canonical(&ours(sol));

        if let Some((_, reason)) = KNOWN_DIVERGENT.iter().find(|(f, _)| *f == rel) {
            // Still divergent either way: solc refuses the file, or produces a
            // different ABI. Only full agreement means the entry is stale.
            if let Ok(expected) = solc_abi_union(sol) {
                assert_ne!(
                    canonical(&expected),
                    actual,
                    "{rel} now matches solc; remove it from KNOWN_DIVERGENT ({reason})"
                );
            }
            divergent_seen.insert(rel);
            continue;
        }

        let expected = match solc_abi_union(sol) {
            Ok(items) => canonical(&items),
            Err(diagnostics) => {
                mismatches.push(format!("{rel}\n  solc refused the file:\n{diagnostics}"));
                continue;
            }
        };

        if expected != actual {
            let missing: Vec<_> = expected.difference(&actual).collect();
            let extra: Vec<_> = actual.difference(&expected).collect();
            mismatches.push(format!(
                "{rel}\n  solc has, we don't: {missing:#?}\n  we have, solc doesn't: {extra:#?}"
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "ABI differs from solc:\n{}",
        mismatches.join("\n")
    );
    assert_eq!(
        divergent_seen.len(),
        KNOWN_DIVERGENT.len(),
        "stale KNOWN_DIVERGENT entry"
    );
    assert_eq!(
        not_solidity_seen.len(),
        NOT_SOLIDITY.len(),
        "stale NOT_SOLIDITY entry"
    );
}
