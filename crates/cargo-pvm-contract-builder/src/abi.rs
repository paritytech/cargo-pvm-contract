use anyhow::{Context, Result};
use std::{env, fs, path::Path, process::Command};
use toml_edit::DocumentMut;

// Re-export ABI types from the canonical definitions in pvm-contract-types.
pub use pvm_contract_types::{AbiEventParam, AbiItem, AbiJson, AbiParam, parse_type_str};

pub fn generate_abi_for_bin(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<AbiJson>> {
    generate_abi_via_feature(manifest_dir, bin_name, target_root, features)
}

/// Generate the storage layout JSON for a binary, if the contract declares
/// `#[slot(N)]` fields. Returns the raw `serde_json::Value` of the
/// `storageLayout` object, or `None` when no storage is declared.
///
/// Detection is a simple source-level check for `#[slot(`. When slot fields
/// are present, the `#[contract]` macro generates an abi-gen `main()` that
/// outputs the layout. If the abi-gen binary fails with "main function not
/// found", we treat it as "no storage layout" (the slot attr may have been
/// outside a `#[contract]` module). All other failures propagate normally.
pub fn generate_storage_layout_for_bin(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    let source_path = resolve_bin_source_path(manifest_dir, bin_name)?;
    if !source_path.exists()
        || !has_slot_fields(
            &fs::read_to_string(&source_path)
                .with_context(|| format!("Failed to read {}", source_path.display()))?,
        )
    {
        return Ok(None);
    }

    let stdout = match run_abi_gen_binary(manifest_dir, bin_name, target_root, features) {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(None),
        Err(e) => {
            let msg = format!("{e:?}");
            if msg.contains("main function not found")
                || msg.contains("main` function not found")
                || msg.contains("does not contain this feature")
            {
                return Ok(None);
            }
            return Err(e).context("Failed to generate storage layout via abi-gen");
        }
    };
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(None);
    }

    let value: serde_json::Value =
        serde_json::from_str(stdout).context("Failed to parse storage layout JSON from abi-gen")?;

    // The output is either just the storage layout (sol path) or a combined
    // object with "storageLayout" field (non-sol path).
    Ok(value
        .get("storageLayout")
        .cloned()
        .or_else(|| value.get("storage").map(|_| value.clone())))
}

fn get_host_triple() -> Result<String> {
    if let Ok(host) = env::var("HOST") {
        return Ok(host);
    }
    // Fallback: parse `rustc -vV` output
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("Failed to execute rustc -vV")?;
    if !output.status.success() {
        anyhow::bail!("rustc -vV failed");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(triple) = line.strip_prefix("host: ") {
            return Ok(triple.trim().to_string());
        }
    }
    anyhow::bail!("Could not determine host triple from rustc -vV")
}

fn generate_abi_via_feature(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<AbiJson>> {
    let source_path = resolve_bin_source_path(manifest_dir, bin_name)?;
    if !source_path.exists() {
        return Ok(None);
    }

    let source_content = fs::read_to_string(&source_path)
        .with_context(|| format!("Failed to read {}", source_path.display()))?;

    if let Some(sol_path) = extract_sol_path_from_source(&source_content) {
        let sol_full_path = manifest_dir.join(sol_path);
        return generate_abi_from_sol(&sol_full_path);
    }

    // Non-.sol path: ABI generation requires the `#[contract]` macro.
    if !has_contract_macro(&source_content) {
        return Ok(None);
    }

    // The abi-gen binary outputs either a bare ABI array (no storage) or
    // {"abi": [...], "storageLayout": {...}} (with storage). Extract
    // just the ABI items either way.
    let stdout = run_abi_gen_binary(manifest_dir, bin_name, target_root, features)?
        .context("abi-gen binary produced no output")?;

    let value: serde_json::Value =
        serde_json::from_str(&stdout).context("Failed to parse abi-gen output as JSON")?;

    let abi_value = match value.get("abi") {
        Some(v) => v.clone(),
        None => value, // bare array
    };

    let abi: AbiJson =
        serde_json::from_value(abi_value).context("Failed to parse ABI from abi-gen output")?;

    Ok(Some(abi))
}

/// Run the abi-gen binary and return its raw stdout.
fn run_abi_gen_binary(
    manifest_dir: &Path,
    bin_name: &str,
    target_root: Option<&Path>,
    features: Option<&str>,
) -> Result<Option<String>> {
    let target_dir = match target_root {
        Some(root) => root.join("abi-gen-target"),
        None => super::get_target_root().join("abi-gen-target"),
    };
    let manifest_path = manifest_dir.join("Cargo.toml");
    let host = get_host_triple()?;

    let has_toolchain_file = manifest_dir.join("rust-toolchain.toml").exists()
        || manifest_dir.join("rust-toolchain").exists();

    let mut cmd = Command::new("cargo");
    cmd.current_dir(manifest_dir)
        .env_remove("CARGO")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC")
        .env("CARGO_TARGET_DIR", &target_dir);

    if has_toolchain_file {
        cmd.env_remove("RUSTUP_TOOLCHAIN");
    }

    // Combine `abi-gen` with any user-supplied features into a single
    // `--features` argument (cargo accepts comma-separated lists).
    let combined_features = match features {
        Some(list) if !list.trim().is_empty() => format!("abi-gen,{list}"),
        _ => "abi-gen".to_string(),
    };

    let output = cmd
        .env(super::INTERNAL_BUILD_ENV, "1")
        .arg("run")
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--target")
        .arg(&host)
        .arg("--config")
        .arg(r#"unstable.build-std=["std","core","alloc"]"#)
        .arg("--features")
        .arg(&combined_features)
        .arg("--bin")
        .arg(bin_name)
        .output()
        .context("Failed to execute abi-gen compile and run")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ABI generation via abi-gen feature failed:\n{stderr}");
    }

    let stdout_str =
        String::from_utf8(output.stdout).context("ABI generation output is not valid UTF-8")?;
    let trimmed = stdout_str.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

/// Locate the `#[contract]` attribute in the source, if present, and return
/// a reference for further inspection. Used to gate ABI generation, which
/// runs only when the macro is actually present (DSL contracts skip it).
/// Matches every form the user might write: `#[contract]`, `#[contract(...)]`,
/// `#[pvm_contract_sdk::contract]`, `#[pvm_contract_sdk::contract(...)]`,
/// `#[pvm_contract_macros::contract]`, `#[pvm_contract_macros::contract(...)]`.
fn find_contract_attr(items: &[syn::Item]) -> Option<&syn::Attribute> {
    for item in items {
        for attr in item_attrs(item) {
            if attr
                .path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "contract")
            {
                return Some(attr);
            }
        }
        if let syn::Item::Mod(m) = item
            && let Some((_, nested)) = &m.content
            && let Some(found) = find_contract_attr(nested)
        {
            return Some(found);
        }
    }
    None
}

/// Return the attributes attached to any top-level Rust item (`struct`, `fn`,
/// `mod`, `impl`, etc.). The `_ => &[]` arm handles `syn::Item`'s
/// `#[non_exhaustive]` declaration so future variants don't break the build.
fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::ExternCrate(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::ForeignMod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::TraitAlias(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Union(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

/// Detect whether the source uses the `#[contract]` attribute macro. Used to
/// skip ABI generation for DSL-based contracts that don't use the macro.
///
/// Parses the source via `syn` so that `#[contract]`-shaped text in comments
/// or string literals doesn't trip detection.
pub(crate) fn has_contract_macro(source: &str) -> bool {
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    find_contract_attr(&file.items).is_some()
}

/// Walk for any struct field carrying a `#[slot(...)]` attribute, recursing
/// into `mod` contents.
fn any_struct_field_has_slot_attr(items: &[syn::Item]) -> bool {
    for item in items {
        match item {
            syn::Item::Struct(s) => {
                if let syn::Fields::Named(named) = &s.fields
                    && named
                        .named
                        .iter()
                        .any(|f| f.attrs.iter().any(|a| a.path().is_ident("slot")))
                {
                    return true;
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, nested)) = &m.content
                    && any_struct_field_has_slot_attr(nested)
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Detect whether the source contains a `#[slot(...)]` attribute on a struct
/// field, which indicates storage fields on the contract struct.
///
/// Parses the source via `syn` so that `#[slot(...)]`-shaped text in comments
/// or string literals doesn't trip detection.
fn has_slot_fields(source: &str) -> bool {
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    any_struct_field_has_slot_attr(&file.items)
}

pub(crate) fn extract_sol_path_from_source(source: &str) -> Option<String> {
    let file = syn::parse_file(source).ok()?;
    let attr = find_contract_attr(&file.items)?;

    // `#[contract]` with no args is `Meta::Path` (no Sol arg).
    // `#[contract(...)]` is `Meta::List`; parse the comma-separated args
    // and return the first literal string ending in `.sol`.
    let args = attr
        .parse_args_with(syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
        .ok()?;

    for expr in &args {
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) = expr
        {
            let path = s.value();
            if path.ends_with(".sol") {
                return Some(path);
            }
        }
    }
    None
}

pub(crate) fn resolve_bin_source_path(
    manifest_dir: &Path,
    bin_name: &str,
) -> Result<std::path::PathBuf> {
    let cargo_toml_path = manifest_dir.join("Cargo.toml");
    let cargo_toml = std::fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("Failed to read {}", cargo_toml_path.display()))?;
    let doc = cargo_toml
        .parse::<DocumentMut>()
        .context("Failed to parse Cargo.toml")?;

    if let Some(bin_array) = doc.get("bin").and_then(|b| b.as_array_of_tables()) {
        for bin in bin_array {
            if bin.get("name").and_then(|n| n.as_str()) == Some(bin_name) {
                if let Some(path) = bin.get("path").and_then(|p| p.as_str()) {
                    return Ok(manifest_dir.join(path));
                }
                return Ok(manifest_dir.join("src/bin").join(format!("{bin_name}.rs")));
            }
        }
    }

    if doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some(bin_name)
    {
        return Ok(manifest_dir.join("src/main.rs"));
    }

    Ok(manifest_dir.join("src/bin").join(format!("{bin_name}.rs")))
}

pub(crate) fn generate_abi_from_sol(sol_path: &Path) -> Result<Option<AbiJson>> {
    let content = std::fs::read_to_string(sol_path)
        .with_context(|| format!("Failed to read sol file: {}", sol_path.display()))?;

    let mut items = Vec::new();
    // Accumulate multiline declarations using balanced-paren detection,
    // matching the approach in pvm-contract-macros/src/solidity.rs.
    let mut pending: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some(ref mut acc) = pending {
            acc.push(' ');
            acc.push_str(line);
            if has_balanced_parens(acc) {
                try_parse_decl(acc, &mut items);
                pending = None;
            }
        } else if line.starts_with("function ")
            || line.starts_with("constructor")
            || line.starts_with("error ")
            || line.starts_with("event ")
            || line.starts_with("receive(")
            || line.starts_with("receive ")
        {
            if has_balanced_parens(line) {
                try_parse_decl(line, &mut items);
            } else {
                pending = Some(line.to_string());
            }
        }
    }

    if items.is_empty() {
        return Ok(None);
    }

    // Append framework-level parameterless custom errors, unless the .sol
    // interface already defines an error with the same name.
    for name in pvm_contract_types::framework_errors::NAMES {
        let already_defined = items
            .iter()
            .any(|item| matches!(item, AbiItem::Error { name: n, .. } if n == name));
        if !already_defined {
            items.push(AbiItem::Error {
                name: name.to_string(),
                inputs: vec![],
            });
        }
    }

    Ok(Some(AbiJson(items)))
}

/// Try to parse a complete declaration line as function, constructor, or error.
fn try_parse_decl(line: &str, items: &mut Vec<AbiItem>) {
    if line.starts_with("function ")
        && let Some(func) = parse_sol_function_line(line)
    {
        items.push(func);
    } else if line.starts_with("constructor")
        && let Some(ctor) = parse_sol_constructor_line(line)
    {
        items.push(ctor);
    } else if line.starts_with("error ")
        && let Some(err) = parse_sol_error_line(line)
    {
        items.push(err);
    } else if line.starts_with("event ")
        && let Some(evt) = parse_sol_event_line(line)
    {
        items.push(evt);
    } else if (line.starts_with("receive(") || line.starts_with("receive "))
        && let Some(recv) = parse_sol_receive_line(line)
    {
        items.push(recv);
    }
}

fn parse_sol_receive_line(line: &str) -> Option<AbiItem> {
    let rest = line.strip_prefix("receive")?;
    let after = rest.trim_start();
    if !after.starts_with('(') {
        return None;
    }
    Some(AbiItem::Receive {
        state_mutability: Some("payable".to_string()),
    })
}

/// Check whether all parentheses in `s` are balanced.
fn has_balanced_parens(s: &str) -> bool {
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// Find the index of the closing `)` that matches the `(` at `start`.
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, ch) in s[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split parameters at top-level commas, respecting nested parens and brackets.
fn split_top_level(params_str: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut depth = 0;
    let mut current = String::new();

    for ch in params_str.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    params.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        params.push(current.trim().to_string());
    }

    params
}

pub(crate) fn parse_sol_function_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("function ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = find_matching_paren(line, paren_start)?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_params(params_str);

    let outputs = if let Some(returns_idx) = line.find("returns") {
        let after_returns = &line[returns_idx + 7..];
        if let Some(start) = after_returns.find('(') {
            let abs_start = returns_idx + 7 + start;
            if let Some(end) = find_matching_paren(line, abs_start) {
                parse_sol_params(&line[abs_start + 1..end])
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let state_mutability = line
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .find(|tok| matches!(*tok, "view" | "pure" | "payable"))
        .unwrap_or("nonpayable")
        .to_string();

    Some(AbiItem::Function {
        name,
        inputs,
        outputs,
        state_mutability: Some(state_mutability),
    })
}

fn parse_sol_constructor_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("constructor")?.trim();
    let paren_start = line.find('(')?;
    let paren_end = find_matching_paren(line, paren_start)?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_params(params_str);

    let state_mutability = if line.contains(" payable") {
        "payable"
    } else {
        "nonpayable"
    }
    .to_string();

    Some(AbiItem::Constructor {
        inputs,
        state_mutability: Some(state_mutability),
    })
}

fn parse_sol_error_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("error ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = find_matching_paren(line, paren_start)?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_params(params_str);

    Some(AbiItem::Error { name, inputs })
}

fn parse_sol_event_line(line: &str) -> Option<AbiItem> {
    let line = line.strip_prefix("event ")?.trim();

    let paren_start = line.find('(')?;
    let name = line[..paren_start].trim().to_string();

    let paren_end = find_matching_paren(line, paren_start)?;
    let params_str = &line[paren_start + 1..paren_end];
    let inputs = parse_sol_event_params(params_str);

    let anonymous = line[paren_end..].contains("anonymous");

    Some(AbiItem::Event {
        name,
        inputs,
        anonymous,
    })
}

fn parse_sol_event_params(params_str: &str) -> Vec<AbiEventParam> {
    if params_str.trim().is_empty() {
        return vec![];
    }

    split_top_level(params_str)
        .into_iter()
        .filter_map(|p| {
            let p = p.trim().to_string();
            let parts: Vec<&str> = p.split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }
            let raw_type = parts[0];
            let indexed = parts.contains(&"indexed");
            let name = parts[1..]
                .iter()
                .find(|s| !matches!(**s, "indexed" | "memory" | "calldata" | "storage"))
                .map(|s| s.to_string())
                .unwrap_or_default();
            let expanded = parse_type_str(&name, raw_type);
            Some(AbiEventParam {
                name: expanded.name,
                param_type: expanded.param_type,
                components: expanded.components,
                indexed,
            })
        })
        .collect()
}

pub(crate) fn parse_sol_params(params_str: &str) -> Vec<AbiParam> {
    if params_str.trim().is_empty() {
        return vec![];
    }

    split_top_level(params_str)
        .into_iter()
        .filter_map(|p| {
            let p = p.trim().to_string();
            let parts: Vec<&str> = p.split_whitespace().collect();
            if parts.is_empty() {
                return None;
            }
            let raw_type = parts[0].to_string();
            let name = parts[1..]
                .iter()
                .find(|s| !matches!(**s, "memory" | "calldata" | "storage"))
                .map(|s| s.to_string())
                .unwrap_or_default();
            Some(parse_type_str(&name, &raw_type))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    // --- extract_sol_path_from_source ---

    #[test]
    fn extract_sol_path_valid() {
        let source = r#"
            #[pvm_contract_macros::contract("MyToken.sol", buffer = 256)]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("MyToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_with_directory() {
        let source = r#"
            #[contract("interfaces/IToken.sol")]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("interfaces/IToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_no_contract_attr() {
        assert_eq!(extract_sol_path_from_source("fn main() {}"), None);
    }

    #[test]
    fn extract_sol_path_non_sol_extension() {
        let source = r#"
            #[contract("MyToken.json")]
            mod c {}
        "#;
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    #[test]
    fn extract_sol_path_with_whitespace_after_paren() {
        let source = r#"
            #[contract( "MyToken.sol" )]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("MyToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_with_newline_formatting() {
        let source = "
            #[contract(\n    \"MyToken.sol\"\n)]
            mod c {}
        ";
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("MyToken.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_missing_closing_quote() {
        // Syntactically broken source: `syn::parse_file` returns Err and the
        // function returns the conservative `None` rather than guessing.
        let source = r#"#[contract("MyToken.sol)]"#;
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    #[test]
    fn extract_sol_path_ignores_comment_before_real_attribute() {
        // The commented-out attribute must not shadow the real one below it.
        let source = r#"
            // #[pvm_contract_sdk::contract("Wrong.sol")]
            #[pvm_contract_sdk::contract("Right.sol")]
            mod c {}
        "#;
        assert_eq!(
            extract_sol_path_from_source(source),
            Some("Right.sol".to_string())
        );
    }

    #[test]
    fn extract_sol_path_with_allocator_only() {
        // `#[contract(allocator = "pico")]` has no .sol arg, must return None.
        let source = r#"
            #[pvm_contract_sdk::contract(allocator = "pico")]
            mod c {}
        "#;
        assert_eq!(extract_sol_path_from_source(source), None);
    }

    // --- has_contract_macro ---

    #[test]
    fn has_contract_macro_with_args() {
        let source = r#"
            #[pvm_contract_macros::contract(allocator = "pico")]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_with_sol_path() {
        let source = r#"
            #[pvm_contract_macros::contract("MyToken.sol")]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_no_args() {
        let source = r#"
            #[pvm_contract_macros::contract]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_dsl_binary() {
        let source = r#"use pvm_contract_builder_dsl::{ContractBuilder, solidity_selector};"#;
        assert!(!has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_ignores_comment() {
        // A `#[contract(...)]`-shape inside a comment is not a real attribute.
        let source = r#"
            // #[pvm_contract_sdk::contract("Foo.sol")]
            fn main() {}
        "#;
        assert!(!has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_handles_bare_path_form() {
        // `#[contract]` (no `::`-prefix) must still match.
        let source = r#"
            use pvm_contract_sdk::contract;
            #[contract]
            mod c {}
        "#;
        assert!(has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_returns_false_on_syntax_error() {
        // Source that doesn't parse: function returns the conservative default.
        let source = r#"#[contract(unclosed"#;
        assert!(!has_contract_macro(source));
    }

    #[test]
    fn has_contract_macro_finds_attribute_inside_nested_mod() {
        let source = r#"
            mod outer {
                #[pvm_contract_sdk::contract("Foo.sol")]
                pub mod inner {}
            }
        "#;
        assert!(has_contract_macro(source));
    }

    // --- has_slot_fields ---

    #[test]
    fn has_slot_fields_detects_real_attribute() {
        let source = r#"
            struct S {
                #[slot(0)]
                x: u32,
            }
        "#;
        assert!(has_slot_fields(source));
    }

    #[test]
    fn has_slot_fields_ignores_comment() {
        // a `#[slot(...)]` shape inside a comment must not trip detection.
        let source = r#"
            struct S {
                // #[slot(0)]
                x: u32,
            }
        "#;
        assert!(!has_slot_fields(source));
    }

    #[test]
    fn has_slot_fields_finds_attribute_inside_nested_mod() {
        let source = r#"
            mod contract {
                pub struct S {
                    #[slot(0)]
                    total: u32,
                }
            }
        "#;
        assert!(has_slot_fields(source));
    }

    #[test]
    fn has_slot_fields_returns_false_when_absent() {
        let source = r#"
            struct S {
                x: u32,
            }
        "#;
        assert!(!has_slot_fields(source));
    }

    // --- parse_sol_params ---

    #[test]
    fn parse_params_empty() {
        assert_eq!(parse_sol_params(""), Vec::<AbiParam>::new());
    }

    #[test]
    fn parse_params_whitespace_only() {
        assert_eq!(parse_sol_params("   "), Vec::<AbiParam>::new());
    }

    #[test]
    fn parse_params_single_with_name() {
        let params = parse_sol_params("uint256 amount");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "uint256");
        assert_eq!(params[0].name, "amount");
    }

    #[test]
    fn parse_params_single_type_only() {
        let params = parse_sol_params("uint256");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "uint256");
        assert_eq!(params[0].name, "");
    }

    #[test]
    fn parse_params_multiple() {
        let params = parse_sol_params("address to, uint256 amount");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].param_type, "address");
        assert_eq!(params[0].name, "to");
        assert_eq!(params[1].param_type, "uint256");
        assert_eq!(params[1].name, "amount");
    }

    // --- parse_sol_function_line ---

    #[test]
    fn parse_function_simple_transfer() {
        let item =
            parse_sol_function_line("function transfer(address to, uint256 amount) external")
                .unwrap();
        assert_eq!(
            item,
            AbiItem::Function {
                name: "transfer".to_string(),
                inputs: vec![
                    AbiParam {
                        name: "to".to_string(),
                        param_type: "address".to_string(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "amount".to_string(),
                        param_type: "uint256".to_string(),
                        components: vec![],
                    },
                ],
                outputs: vec![],
                state_mutability: Some("nonpayable".to_string()),
            }
        );
    }

    #[test]
    fn parse_function_view_with_returns() {
        let item = parse_sol_function_line(
            "function balanceOf(address account) external view returns (uint256)",
        )
        .unwrap();
        assert_eq!(
            item,
            AbiItem::Function {
                name: "balanceOf".to_string(),
                inputs: vec![AbiParam {
                    name: "account".to_string(),
                    param_type: "address".to_string(),
                    components: vec![],
                }],
                outputs: vec![AbiParam {
                    name: "".to_string(),
                    param_type: "uint256".to_string(),
                    components: vec![],
                }],
                state_mutability: Some("view".to_string()),
            }
        );
    }

    #[test]
    fn parse_function_no_params() {
        let item =
            parse_sol_function_line("function totalSupply() external view returns (uint256)")
                .unwrap();
        assert_eq!(
            item,
            AbiItem::Function {
                name: "totalSupply".to_string(),
                inputs: vec![],
                outputs: vec![AbiParam {
                    name: "".to_string(),
                    param_type: "uint256".to_string(),
                    components: vec![],
                }],
                state_mutability: Some("view".to_string()),
            }
        );
    }

    #[test]
    fn parse_function_pure_mutability() {
        let item =
            parse_sol_function_line("function add(uint256 a, uint256 b) pure returns (uint256)")
                .unwrap();
        if let AbiItem::Function {
            state_mutability, ..
        } = &item
        {
            assert_eq!(state_mutability.as_deref(), Some("pure"));
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_payable_mutability() {
        let item =
            parse_sol_function_line("function deposit() external payable returns (bool)").unwrap();
        if let AbiItem::Function {
            state_mutability, ..
        } = &item
        {
            assert_eq!(state_mutability.as_deref(), Some("payable"));
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_payable_with_trailing_semicolon() {
        let item = parse_sol_function_line("function deposit() external payable;").unwrap();
        if let AbiItem::Function {
            state_mutability, ..
        } = &item
        {
            assert_eq!(state_mutability.as_deref(), Some("payable"));
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_view_with_trailing_semicolon() {
        let item = parse_sol_function_line("function owner() external view;").unwrap();
        if let AbiItem::Function {
            state_mutability, ..
        } = &item
        {
            assert_eq!(state_mutability.as_deref(), Some("view"));
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_no_returns() {
        let item = parse_sol_function_line("function setOwner(address newOwner) external").unwrap();
        if let AbiItem::Function { outputs, .. } = &item {
            assert!(outputs.is_empty());
        } else {
            panic!("expected Function");
        }
    }

    #[test]
    fn parse_function_not_a_function() {
        assert!(parse_sol_function_line("event Transfer(address,address,uint256)").is_none());
    }

    // --- generate_abi_from_sol (uses temp files) ---

    #[test]
    fn generate_abi_from_sol_valid_interface() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("IToken.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"// SPDX-License-Identifier: MIT
interface IToken {{
    function totalSupply() external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 7);
        assert_eq!(arr[0]["name"], "totalSupply");
        assert_eq!(arr[1]["name"], "transfer");
        assert_eq!(arr[2]["name"], "InvalidCalldata");
        assert_eq!(arr[2]["type"], "error");
        assert_eq!(arr[3]["name"], "CalldataTooLarge");
        assert_eq!(arr[3]["type"], "error");
        assert_eq!(arr[4]["name"], "NoSelector");
        assert_eq!(arr[4]["type"], "error");
        assert_eq!(arr[5]["name"], "UnknownSelector");
        assert_eq!(arr[5]["type"], "error");
        assert_eq!(arr[6]["name"], "NonPayableValueReceived");
        assert_eq!(arr[6]["type"], "error");
    }

    #[test]
    fn generate_abi_from_sol_empty_file() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Empty.sol");
        std::fs::write(&sol_path, "// empty").unwrap();

        assert!(generate_abi_from_sol(&sol_path).unwrap().is_none());
    }

    #[test]
    fn generate_abi_from_sol_no_functions() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("IEmpty.sol");
        std::fs::write(&sol_path, "interface IEmpty {}").unwrap();

        assert!(generate_abi_from_sol(&sol_path).unwrap().is_none());
    }

    // --- resolve_bin_source_path (uses temp dirs with Cargo.toml) ---

    #[test]
    fn resolve_bin_path_explicit_bin_entry() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "myproject"
version = "0.1.0"

[[bin]]
name = "mybin"
path = "src/custom.rs"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/custom.rs"));
    }

    #[test]
    fn resolve_bin_path_bin_entry_without_path() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "myproject"
version = "0.1.0"

[[bin]]
name = "mybin"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/bin/mybin.rs"));
    }

    #[test]
    fn resolve_bin_path_package_name_match() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "mybin"
version = "0.1.0"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/main.rs"));
    }

    #[test]
    fn resolve_bin_path_fallback() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"[package]
name = "other"
version = "0.1.0"
"#,
        )
        .unwrap();

        let path = resolve_bin_source_path(dir.path(), "mybin").unwrap();
        assert_eq!(path, dir.path().join("src/bin/mybin.rs"));
    }

    #[test]
    fn parse_params_strips_data_location_qualifiers() {
        let params = parse_sol_params("string calldata s, uint256[] memory arr");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].param_type, "string");
        assert_eq!(params[0].name, "s");
        assert_eq!(params[1].param_type, "uint256[]");
        assert_eq!(params[1].name, "arr");
    }

    #[test]
    fn parse_params_strips_qualifier_without_name() {
        let params = parse_sol_params("string memory");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "string");
        assert_eq!(params[0].name, "");
    }

    #[test]
    fn parse_function_with_tuple_param() {
        let item = parse_sol_function_line(
            "function foo((address,uint256) param) external returns (bool)",
        )
        .unwrap();
        if let AbiItem::Function {
            name,
            inputs,
            outputs,
            ..
        } = &item
        {
            assert_eq!(name, "foo");
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0].param_type, "tuple");
            assert_eq!(inputs[0].components.len(), 2);
            assert_eq!(inputs[0].components[0].param_type, "address");
            assert_eq!(inputs[0].components[1].param_type, "uint256");
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0].param_type, "bool");
        } else {
            panic!("expected Function");
        }
    }

    // --- Error parsing ---

    #[test]
    fn parse_error_with_params() {
        assert_eq!(
            parse_sol_error_line(
                "error InsufficientBalance(address account, uint256 required, uint256 available);",
            )
            .unwrap(),
            AbiItem::Error {
                name: "InsufficientBalance".to_string(),
                inputs: vec![
                    AbiParam {
                        name: "account".to_string(),
                        param_type: "address".to_string(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "required".to_string(),
                        param_type: "uint256".to_string(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "available".to_string(),
                        param_type: "uint256".to_string(),
                        components: vec![],
                    },
                ],
            }
        );
    }

    #[test]
    fn parse_error_no_params() {
        assert_eq!(
            parse_sol_error_line("error Unauthorized();").unwrap(),
            AbiItem::Error {
                name: "Unauthorized".to_string(),
                inputs: vec![],
            }
        );
    }

    #[test]
    fn generate_abi_from_sol_includes_errors() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("MyToken.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"interface IToken {{
    function transfer(address to, uint256 amount) external;
    error InsufficientBalance(address account, uint256 required);
    error Unauthorized();
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 8);
        assert_eq!(arr[0]["name"], "transfer");
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[1]["name"], "InsufficientBalance");
        assert_eq!(arr[1]["type"], "error");
        assert_eq!(arr[2]["name"], "Unauthorized");
        assert_eq!(arr[2]["type"], "error");
        assert_eq!(arr[3]["name"], "InvalidCalldata");
        assert_eq!(arr[3]["type"], "error");
        assert_eq!(arr[4]["name"], "CalldataTooLarge");
        assert_eq!(arr[4]["type"], "error");
        assert_eq!(arr[5]["name"], "NoSelector");
        assert_eq!(arr[5]["type"], "error");
        assert_eq!(arr[6]["name"], "UnknownSelector");
        assert_eq!(arr[6]["type"], "error");
        assert_eq!(arr[7]["name"], "NonPayableValueReceived");
        assert_eq!(arr[7]["type"], "error");
    }

    // --- Multiline declaration support ---

    #[test]
    fn generate_abi_from_sol_multiline_function() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Multi.sol");
        std::fs::write(
            &sol_path,
            "interface Multi {\n    function transfer(\n        address to,\n        uint256 amount\n    ) external;\n}",
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let func = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Function { name, .. } if name == "transfer"))
            .expect("should parse multiline function");
        if let AbiItem::Function { inputs, .. } = func {
            assert_eq!(inputs.len(), 2);
            assert_eq!(inputs[0].param_type, "address");
            assert_eq!(inputs[0].name, "to");
            assert_eq!(inputs[1].param_type, "uint256");
            assert_eq!(inputs[1].name, "amount");
        }
    }

    // --- Constructor parsing ---

    #[test]
    fn parse_constructor_no_params() {
        let item = parse_sol_constructor_line("constructor() public").unwrap();
        assert_eq!(
            item,
            AbiItem::Constructor {
                inputs: vec![],
                state_mutability: Some("nonpayable".to_string()),
            }
        );
    }

    #[test]
    fn parse_constructor_with_params() {
        let item =
            parse_sol_constructor_line("constructor(address owner, uint256 supply) public payable")
                .unwrap();
        if let AbiItem::Constructor {
            inputs,
            state_mutability,
        } = &item
        {
            assert_eq!(inputs.len(), 2);
            assert_eq!(inputs[0].param_type, "address");
            assert_eq!(inputs[0].name, "owner");
            assert_eq!(inputs[1].param_type, "uint256");
            assert_eq!(inputs[1].name, "supply");
            assert_eq!(state_mutability.as_deref(), Some("payable"));
        } else {
            panic!("expected Constructor");
        }
    }

    // --- Tuple type expansion in parse_sol_params ---

    #[test]
    fn parse_params_tuple_becomes_components() {
        let params = parse_sol_params("(uint256,address) value");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "tuple");
        assert_eq!(params[0].name, "value");
        assert_eq!(params[0].components.len(), 2);
        assert_eq!(params[0].components[0].param_type, "uint256");
        assert_eq!(params[0].components[1].param_type, "address");
    }

    #[test]
    fn parse_params_tuple_array() {
        let params = parse_sol_params("(uint256,address)[] items");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "tuple[]");
        assert_eq!(params[0].components.len(), 2);
    }

    #[test]
    fn parse_params_nested_tuple() {
        let params = parse_sol_params("((uint64,uint64),(uint64,uint64)) line");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].param_type, "tuple");
        assert_eq!(params[0].components.len(), 2);
        assert_eq!(params[0].components[0].param_type, "tuple");
        assert_eq!(params[0].components[0].components.len(), 2);
    }

    // --- Constructor in generate_abi_from_sol ---

    #[test]
    fn generate_abi_from_sol_includes_constructor() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Token.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"// SPDX-License-Identifier: MIT
interface Token {{
    constructor(address owner, uint256 supply);
    function totalSupply() external view returns (uint256);
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();

        // Find constructor entry
        let ctor = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Constructor { .. }))
            .expect("ABI should include constructor");
        assert_eq!(
            *ctor,
            AbiItem::Constructor {
                inputs: vec![
                    AbiParam {
                        name: "owner".into(),
                        param_type: "address".into(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "supply".into(),
                        param_type: "uint256".into(),
                        components: vec![],
                    },
                ],
                state_mutability: Some("nonpayable".into()),
            }
        );

        // Find function entry
        let func = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Function { name, .. } if name == "totalSupply"))
            .expect("ABI should include totalSupply");
        assert_eq!(
            *func,
            AbiItem::Function {
                name: "totalSupply".into(),
                inputs: vec![],
                outputs: vec![AbiParam {
                    name: "".into(),
                    param_type: "uint256".into(),
                    components: vec![],
                }],
                state_mutability: Some("view".into()),
            }
        );
    }

    #[test]
    fn generate_abi_from_sol_multiline_constructor() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Token.sol");
        std::fs::write(
            &sol_path,
            "interface Token {\n    constructor(\n        address owner,\n        uint256 supply\n    ) payable;\n    function totalSupply() external view returns (uint256);\n}",
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let ctor = abi
            .0
            .iter()
            .find(|item| matches!(item, AbiItem::Constructor { .. }))
            .expect("ABI should include multiline constructor");
        assert_eq!(
            *ctor,
            AbiItem::Constructor {
                inputs: vec![
                    AbiParam {
                        name: "owner".into(),
                        param_type: "address".into(),
                        components: vec![],
                    },
                    AbiParam {
                        name: "supply".into(),
                        param_type: "uint256".into(),
                        components: vec![],
                    },
                ],
                state_mutability: Some("payable".into()),
            }
        );
    }

    // --- has_slot_fields ---

    #[test]
    fn detects_slot_attr() {
        assert!(has_slot_fields(
            "pub struct MyToken { #[slot(0)] total_supply: Lazy<U256> }"
        ));
    }

    #[test]
    fn no_slot_attr_returns_false() {
        assert!(!has_slot_fields("pub struct MyToken;"));
    }

    // --- Event parsing ---

    #[test]
    fn parse_event_with_indexed_params() {
        assert_eq!(
            parse_sol_event_line(
                "event Transfer(address indexed from, address indexed to, uint256 value);"
            )
            .unwrap(),
            AbiItem::Event {
                name: "Transfer".to_string(),
                inputs: vec![
                    AbiEventParam {
                        name: "from".to_string(),
                        param_type: "address".to_string(),
                        components: vec![],
                        indexed: true,
                    },
                    AbiEventParam {
                        name: "to".to_string(),
                        param_type: "address".to_string(),
                        components: vec![],
                        indexed: true,
                    },
                    AbiEventParam {
                        name: "value".to_string(),
                        param_type: "uint256".to_string(),
                        components: vec![],
                        indexed: false,
                    },
                ],
                anonymous: false,
            }
        );
    }

    #[test]
    fn parse_event_no_params() {
        assert_eq!(
            parse_sol_event_line("event Paused();").unwrap(),
            AbiItem::Event {
                name: "Paused".to_string(),
                inputs: vec![],
                anonymous: false,
            }
        );
    }

    #[test]
    fn parse_event_anonymous() {
        let item = parse_sol_event_line("event Debug(uint256 value) anonymous;").unwrap();
        if let AbiItem::Event { anonymous, .. } = &item {
            assert!(anonymous);
        } else {
            panic!("expected Event");
        }
    }

    #[test]
    fn parse_event_not_an_event() {
        assert!(parse_sol_event_line("function transfer(address,uint256)").is_none());
    }

    #[test]
    fn parse_event_with_indexed_tuple_param() {
        assert_eq!(
            parse_sol_event_line("event PointMoved((uint64,uint64) indexed point);").unwrap(),
            AbiItem::Event {
                name: "PointMoved".to_string(),
                inputs: vec![AbiEventParam {
                    name: "point".to_string(),
                    param_type: "tuple".to_string(),
                    components: vec![
                        AbiParam {
                            name: "".to_string(),
                            param_type: "uint64".to_string(),
                            components: vec![],
                        },
                        AbiParam {
                            name: "".to_string(),
                            param_type: "uint64".to_string(),
                            components: vec![],
                        },
                    ],
                    indexed: true,
                }],
                anonymous: false,
            }
        );
    }

    #[test]
    fn generate_abi_from_sol_includes_events() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Events.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"interface IEvents {{
    function setValue(uint256 val) external;
    event ValueChanged(address indexed who, uint256 oldValue, uint256 newValue);
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();

        let event = arr.iter().find(|item| item["type"] == "event").unwrap();
        assert_eq!(event["name"], "ValueChanged");
        assert_eq!(event["anonymous"], false);

        let inputs = event["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0]["name"], "who");
        assert_eq!(inputs[0]["type"], "address");
        assert_eq!(inputs[0]["indexed"], true);
        assert_eq!(inputs[1]["name"], "oldValue");
        assert_eq!(inputs[1]["type"], "uint256");
        assert_eq!(inputs[1]["indexed"], false);
        assert_eq!(inputs[2]["name"], "newValue");
        assert_eq!(inputs[2]["type"], "uint256");
        assert_eq!(inputs[2]["indexed"], false);
    }

    #[test]
    fn generate_abi_from_sol_parses_multiline_events() {
        let dir = TempDir::new().unwrap();
        let sol_path = dir.path().join("Events.sol");
        let mut f = std::fs::File::create(&sol_path).unwrap();
        writeln!(
            f,
            r#"interface IEvents {{
    event Transfer(
        address indexed from,
        address indexed to,
        uint256 value
    );
}}"#
        )
        .unwrap();

        let abi = generate_abi_from_sol(&sol_path).unwrap().unwrap();
        let json = serde_json::to_value(&abi).unwrap();
        let arr = json.as_array().unwrap();

        let event = arr.iter().find(|item| item["type"] == "event").unwrap();
        assert_eq!(event["name"], "Transfer");

        let inputs = event["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs[0]["name"], "from");
        assert_eq!(inputs[0]["indexed"], true);
        assert_eq!(inputs[1]["name"], "to");
        assert_eq!(inputs[1]["indexed"], true);
        assert_eq!(inputs[2]["name"], "value");
        assert_eq!(inputs[2]["indexed"], false);
    }
}
