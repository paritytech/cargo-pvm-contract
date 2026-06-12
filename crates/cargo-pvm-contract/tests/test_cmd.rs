use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "reason")]
enum BuildJsonLine {
    #[serde(rename = "cargo-pvm-contract-build-plan")]
    BuildPlan {
        schema_version: u64,
        total: u64,
        unit: String,
    },
    #[serde(rename = "compiler-artifact")]
    CompilerArtifact,
    #[serde(rename = "build-finished")]
    BuildFinished,
    #[serde(other)]
    Other,
}

#[derive(Default)]
struct BuildJsonSummary {
    plan: Option<BuildPlanSummary>,
    compiler_artifacts: u64,
    build_finished: u64,
    json_lines: u64,
}

struct BuildPlanSummary {
    schema_version: u64,
    total: u64,
    unit: String,
}

impl BuildJsonSummary {
    fn record_line(mut self, line: &str) -> Self {
        let line: BuildJsonLine = serde_json::from_str(line).expect("stdout line is JSON");
        self.json_lines += 1;
        match line {
            BuildJsonLine::BuildPlan {
                schema_version,
                total,
                unit,
            } => {
                self.plan = Some(BuildPlanSummary {
                    schema_version,
                    total,
                    unit,
                });
            }
            BuildJsonLine::CompilerArtifact => self.compiler_artifacts += 1,
            BuildJsonLine::BuildFinished => self.build_finished += 1,
            BuildJsonLine::Other => {}
        }
        self
    }

    fn plan(&self) -> &BuildPlanSummary {
        self.plan
            .as_ref()
            .expect("stdout should include a cargo-pvm-contract-build-plan line")
    }

    fn assert_consistent(&self) {
        assert!(self.json_lines > 0, "stdout should include Cargo JSON");
        assert!(
            self.compiler_artifacts > 0,
            "stdout should include cargo compiler-artifact JSON lines"
        );
        assert_eq!(
            self.plan().total,
            self.compiler_artifacts,
            "build plan total should match streamed compiler-artifact count"
        );
    }

    fn snapshot(&self, project_dir: &Path, binary_name: &str) -> serde_json::Value {
        let plan = self.plan();
        let polkavm_path = project_dir
            .join("target")
            .join("release")
            .join(format!("{binary_name}.polkavm"));
        let abi_path = project_dir
            .join("target")
            .join("release")
            .join(format!("{binary_name}.abi.json"));

        serde_json::json!({
            "build_plan": {
                "reason": "cargo-pvm-contract-build-plan",
                "schema_version": plan.schema_version,
                "total": "<matches streamed compiler-artifact count>",
                "unit": &plan.unit,
            },
            "cargo_stream": {
                "compiler_artifacts": "<matches build_plan.total>",
                "build_finished": self.build_finished,
            },
            "artifacts": {
                "polkavm": polkavm_path.exists(),
                "abi_json": abi_path.exists(),
            },
        })
    }
}

fn workspace_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scaffold_example(temp_dir: &TempDir, name: &str, example: &str, api_style: &str) -> PathBuf {
    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("init")
        .arg("--init-type")
        .arg("example")
        .arg("--example")
        .arg(example)
        .arg("--api-style")
        .arg(api_style)
        .arg("--name")
        .arg(name)
        .assert()
        .success();

    project_dir
}

fn scaffold_new_contract(
    temp_dir: &TempDir,
    name: &str,
    api_style: &str,
    allocator: Option<&str>,
) -> PathBuf {
    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("init")
        .arg("--init-type")
        .arg("new")
        .arg("--api-style")
        .arg(api_style)
        .arg("--name")
        .arg(name);

    if let Some(alloc) = allocator {
        cmd.arg("--allocator").arg(alloc);
    }

    cmd.assert().success();

    project_dir
}

fn build_project(project_dir: &Path, profile: &str) {
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(project_dir)
        .arg("pvm-contract")
        .arg("build");

    if profile == "debug" {
        cmd.arg("--profile").arg("dev");
    }

    let status = cmd.status().expect("run cargo pvm-contract build");
    assert!(
        status.success(),
        "cargo pvm-contract build ({profile}) failed"
    );
}

#[test]
fn build_streams_json_message_format_and_writes_artifacts() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "json-build-output", "macro", None);

    let output = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"))
        .current_dir(&project_dir)
        .arg("pvm-contract")
        .arg("build")
        .arg("--message-format")
        .arg("json,json-diagnostic-rendered-ansi")
        .output()
        .expect("run cargo pvm-contract build --message-format json");

    assert!(
        output.status.success(),
        "cargo pvm-contract build --message-format json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let summary = String::from_utf8(output.stdout)
        .expect("stdout is utf-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .fold(BuildJsonSummary::default(), BuildJsonSummary::record_line);
    summary.assert_consistent();

    expect_test::expect![[r#"
        {
          "artifacts": {
            "abi_json": true,
            "polkavm": true
          },
          "build_plan": {
            "reason": "cargo-pvm-contract-build-plan",
            "schema_version": 1,
            "total": "<matches streamed compiler-artifact count>",
            "unit": "compiler-artifact"
          },
          "cargo_stream": {
            "build_finished": 1,
            "compiler_artifacts": "<matches build_plan.total>"
          }
        }"#]]
    .assert_eq(
        &serde_json::to_string_pretty(&summary.snapshot(&project_dir, "json-build-output"))
            .expect("serialize normalized build summary"),
    );

    verify_build_artifacts(&project_dir, "json-build-output", "release");
}

fn run_cli_test(project_dir: &Path) {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(project_dir)
        .arg("pvm-contract")
        .arg("test")
        .arg("--")
        .arg("--nocapture")
        .assert()
        .success();
}

fn run_cli_test_with_manifest(manifest_path: &Path, cwd: &Path) {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(cwd)
        .arg("pvm-contract")
        .arg("test")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--")
        .arg("--nocapture")
        .assert()
        .success();
}

fn verify_build_artifacts(project_dir: &Path, binary_name: &str, profile: &str) {
    verify_polkavm_binary(project_dir, binary_name, profile);
    verify_abi_json(project_dir, binary_name, profile);
}

fn verify_polkavm_binary(project_dir: &Path, binary_name: &str, profile: &str) {
    let target_dir = project_dir.join("target").join(profile);
    let polkavm_file = target_dir.join(format!("{binary_name}.polkavm"));
    assert!(
        polkavm_file.exists(),
        "PolkaVM binary not found: {}",
        polkavm_file.display()
    );
}

fn verify_abi_json(project_dir: &Path, binary_name: &str, profile: &str) {
    let target_dir = project_dir.join("target").join(profile);
    let abi_file = target_dir.join(format!("{binary_name}.abi.json"));
    assert!(
        abi_file.exists(),
        "ABI JSON not found: {}",
        abi_file.display()
    );

    let abi_content = std::fs::read_to_string(&abi_file).expect("read ABI file");
    let abi: serde_json::Value = serde_json::from_str(&abi_content).expect("parse ABI JSON");
    assert!(abi.is_array(), "ABI should be an array");
}

fn verify_cargo_toml(project_dir: &Path, use_dsl: bool) {
    let cargo_toml =
        std::fs::read_to_string(project_dir.join("Cargo.toml")).expect("Cargo.toml exists");

    if use_dsl {
        assert!(cargo_toml.contains("pvm-contract-builder-dsl"));
    } else {
        assert!(cargo_toml.contains("pvm-contract-sdk"));
    }
    assert!(cargo_toml.contains("polkavm-derive"));
    assert!(
        !cargo_toml.contains("[build-dependencies]"),
        "Cargo.toml should not contain [build-dependencies]"
    );
}

#[test]
fn mytoken_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-macro-debug", "MyToken", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "mytoken-macro-debug", "debug");
}

#[test]
fn mytoken_macro_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-macro-release", "MyToken", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "mytoken-macro-release", "release");
}

#[test]
fn mytoken_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-dsl-debug", "MyToken", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "mytoken-dsl-debug", "debug");
}

#[test]
fn mytoken_dsl_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "mytoken-dsl-release", "MyToken", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_polkavm_binary(&project_dir, "mytoken-dsl-release", "release");
}

#[test]
fn fibonacci_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-macro-debug", "Fibonacci", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "fibonacci-macro-debug", "debug");
}

#[test]
fn fibonacci_macro_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-macro-release", "Fibonacci", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "fibonacci-macro-release", "release");
}

#[test]
fn fibonacci_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-dsl-debug", "Fibonacci", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "fibonacci-dsl-debug", "debug");
}

#[test]
fn fibonacci_dsl_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "fibonacci-dsl-release", "Fibonacci", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_polkavm_binary(&project_dir, "fibonacci-dsl-release", "release");
}

#[test]
fn new_contract_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-macro-debug", "macro", None);

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "new-macro-debug", "debug");
}

#[test]
fn new_contract_macro_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-macro-release", "macro", None);

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "release");
    verify_build_artifacts(&project_dir, "new-macro-release", "release");
}

#[test]
fn new_contract_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-dsl-debug", "dsl", Some("no-alloc"));

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "new-dsl-debug", "debug");
}

#[test]
fn new_contract_dsl_release() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-dsl-release", "dsl", Some("no-alloc"));

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "release");
    verify_polkavm_binary(&project_dir, "new-dsl-release", "release");
}

#[test]
fn abi_json_has_correct_structure() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "abi-test", "MyToken", "macro");

    build_project(&project_dir, "debug");

    let abi_file = project_dir
        .join("target")
        .join("debug")
        .join("abi-test.abi.json");
    let abi_content = std::fs::read_to_string(&abi_file).expect("read ABI file");
    let abi: Vec<serde_json::Value> = serde_json::from_str(&abi_content).expect("parse ABI JSON");

    let function_names: Vec<&str> = abi
        .iter()
        .filter(|entry| entry.get("type").and_then(|t| t.as_str()) == Some("function"))
        .filter_map(|entry| entry.get("name").and_then(|n| n.as_str()))
        .collect();

    assert!(
        function_names.contains(&"totalSupply"),
        "ABI should contain totalSupply"
    );
    assert!(
        function_names.contains(&"balanceOf"),
        "ABI should contain balanceOf"
    );
    assert!(
        function_names.contains(&"transfer"),
        "ABI should contain transfer"
    );
    assert!(function_names.contains(&"mint"), "ABI should contain mint");
}

#[test]
fn multi_macro_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "multi-macro-debug", "Multi", "macro");

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "multi-macro-debug", "debug");
}

#[test]
fn multi_dsl_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "multi-dsl-debug", "Multi", "dsl");

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "multi-dsl-debug", "debug");
}

#[test]
fn new_contract_dsl_bump_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "new-dsl-bump-debug", "dsl", Some("bump"));

    verify_cargo_toml(&project_dir, true);
    build_project(&project_dir, "debug");
    verify_polkavm_binary(&project_dir, "new-dsl-bump-debug", "debug");
}

#[test]
fn new_contract_macro_bump_debug() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir =
        scaffold_new_contract(&temp_dir, "new-macro-bump-debug", "macro", Some("bump"));

    verify_cargo_toml(&project_dir, false);
    build_project(&project_dir, "debug");
    verify_build_artifacts(&project_dir, "new-macro-bump-debug", "debug");
}

#[test]
fn polkavm_binary_is_valid() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_example(&temp_dir, "polkavm-test", "Fibonacci", "macro");

    build_project(&project_dir, "release");

    let polkavm_file = project_dir
        .join("target")
        .join("release")
        .join("polkavm-test.polkavm");
    let binary = std::fs::read(&polkavm_file).expect("read polkavm file");

    assert!(!binary.is_empty(), "PolkaVM binary should not be empty");
    assert!(
        binary.len() < 100_000,
        "Release binary should be reasonably small (got {} bytes)",
        binary.len()
    );
}

#[test]
fn cli_test_is_end_to_end_for_scaffolded_macro_project() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "cli-test-macro", "macro", None);

    run_cli_test(&project_dir);
}

#[test]
fn cli_test_supports_manifest_path_from_outside_project_dir() {
    let temp_dir = TempDir::new().expect("temp dir");
    let project_dir = scaffold_new_contract(&temp_dir, "cli-test-manifest", "macro", None);
    let manifest_path = project_dir.join("Cargo.toml");

    run_cli_test_with_manifest(&manifest_path, temp_dir.path());
}

/// Remove the standalone `[workspace]` table that the scaffold template adds
/// to each member's `Cargo.toml`, so the crate can be included in a parent
/// workspace. Parsing with `toml_edit` keeps this robust against template
/// whitespace/ordering changes.
fn strip_workspace_table(manifest: &Path) {
    let content = std::fs::read_to_string(manifest).expect("read member Cargo.toml");
    let mut doc: toml_edit::DocumentMut = content.parse().expect("parse member Cargo.toml");
    doc.remove("workspace");
    std::fs::write(manifest, doc.to_string()).expect("write member Cargo.toml");
}

#[test]
fn build_selects_workspace_member_via_package_flag() {
    let temp_dir = TempDir::new().expect("temp dir");

    let pkg_a = scaffold_new_contract(&temp_dir, "ws-pkg-a", "macro", None);
    let pkg_b = scaffold_new_contract(&temp_dir, "ws-pkg-b", "macro", None);

    strip_workspace_table(&pkg_a.join("Cargo.toml"));
    strip_workspace_table(&pkg_b.join("Cargo.toml"));

    let workspace_manifest = temp_dir.path().join("Cargo.toml");
    std::fs::write(
        &workspace_manifest,
        "[workspace]\nresolver = \"2\"\nmembers = [\"ws-pkg-a\", \"ws-pkg-b\"]\n",
    )
    .expect("write workspace Cargo.toml");

    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .arg("pvm-contract")
        .arg("build")
        .arg("--manifest-path")
        .arg(&workspace_manifest)
        .arg("-p")
        .arg("ws-pkg-a");

    let status = cmd.status().expect("run cargo pvm-contract build -p");
    assert!(status.success(), "build -p ws-pkg-a failed");

    let target_release = temp_dir.path().join("target").join("release");
    assert!(
        target_release.join("ws-pkg-a.polkavm").exists(),
        "ws-pkg-a.polkavm should exist at workspace target/release"
    );
    assert!(
        target_release.join("ws-pkg-a.abi.json").exists(),
        "ws-pkg-a.abi.json should exist at workspace target/release"
    );
    assert!(
        !target_release.join("ws-pkg-b.polkavm").exists(),
        "ws-pkg-b.polkavm should NOT exist — only ws-pkg-a was selected"
    );
}

#[test]
fn build_forwards_features_with_package_flag() {
    let temp_dir = TempDir::new().expect("temp dir");

    let pkg = scaffold_new_contract(&temp_dir, "ws-feat-pkg", "macro", None);
    strip_workspace_table(&pkg.join("Cargo.toml"));

    // Add a `gated-build` feature to the member.
    let member_manifest = pkg.join("Cargo.toml");
    let content = std::fs::read_to_string(&member_manifest).expect("read member Cargo.toml");
    let mut doc: toml_edit::DocumentMut = content.parse().expect("parse member Cargo.toml");
    let features = doc
        .entry("features")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .expect("[features] table");
    features.insert("gated-build", toml_edit::value(toml_edit::Array::new()));
    std::fs::write(&member_manifest, doc.to_string()).expect("write member Cargo.toml");

    // Make the contract refuse to compile unless `gated-build` is on. This
    // proves the feature was actually forwarded — without it, the build
    // would hit `compile_error!`. Appended after the scaffold's inner
    // attributes so it doesn't push them past an item.
    let src = pkg.join("src/ws-feat-pkg.rs");
    let mut source = std::fs::read_to_string(&src).expect("read contract source");
    source.push_str(
        "\n#[cfg(not(feature = \"gated-build\"))]\ncompile_error!(\"gated-build feature is required\");\n",
    );
    std::fs::write(&src, source).expect("write contract source");

    let workspace_manifest = temp_dir.path().join("Cargo.toml");
    std::fs::write(
        &workspace_manifest,
        "[workspace]\nresolver = \"2\"\nmembers = [\"ws-feat-pkg\"]\n",
    )
    .expect("write workspace Cargo.toml");

    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .arg("pvm-contract")
        .arg("build")
        .arg("--manifest-path")
        .arg(&workspace_manifest)
        .arg("-p")
        .arg("ws-feat-pkg")
        .arg("--features")
        .arg("gated-build");

    let status = cmd
        .status()
        .expect("run cargo pvm-contract build -p ... --features ...");
    assert!(
        status.success(),
        "build -p ws-feat-pkg --features gated-build failed"
    );

    let target_release = temp_dir.path().join("target").join("release");
    assert!(
        target_release.join("ws-feat-pkg.polkavm").exists(),
        "ws-feat-pkg.polkavm should exist at workspace target/release"
    );
    assert!(
        target_release.join("ws-feat-pkg.abi.json").exists(),
        "ws-feat-pkg.abi.json should exist — proves --features reached the abi-gen invocation too"
    );
}

/// Regression test for paritytech/bugbounty_reports#78: stripping the
/// `#[contract]` macro from a previously-built project must not leave the
/// old `.abi.json` next to the freshly-linked `.polkavm`.
#[test]
fn rebuild_without_contract_macro_clears_stale_abi_json() {
    let temp_dir = TempDir::new().expect("temp dir");
    let name = "stale-abi-test";

    let project_dir = scaffold_new_contract(&temp_dir, name, "macro", None);
    build_project(&project_dir, "release");
    verify_abi_json(&project_dir, name, "release");

    let src_path = project_dir.join("src").join(format!("{name}.rs"));
    let stripped = include_str!("fixtures/no_contract_macro.rs");
    std::fs::write(&src_path, stripped).expect("rewrite source");

    build_project(&project_dir, "release");

    let abi_path = project_dir
        .join("target")
        .join("release")
        .join(format!("{name}.abi.json"));
    assert!(
        !abi_path.exists(),
        "stale .abi.json at {}",
        abi_path.display()
    );
    verify_polkavm_binary(&project_dir, name, "release");
}

/// A no-op source edit must produce a byte-identical `.abi.json`.
/// Guards against a regression where cleanup runs but re-emission doesn't.
#[test]
fn rebuild_with_macro_keeps_abi_byte_stable() {
    let temp_dir = TempDir::new().expect("temp dir");
    let name = "fresh-abi-test";

    let project_dir = scaffold_new_contract(&temp_dir, name, "macro", None);
    build_project(&project_dir, "release");
    verify_abi_json(&project_dir, name, "release");

    let abi_path = project_dir
        .join("target")
        .join("release")
        .join(format!("{name}.abi.json"));
    let abi_v1 = std::fs::read(&abi_path).expect("read v1 abi");

    let src_path = project_dir.join("src").join(format!("{name}.rs"));
    let original = std::fs::read_to_string(&src_path).expect("read source");
    std::fs::write(&src_path, format!("{original}\n// rebuild trigger\n")).expect("write source");

    build_project(&project_dir, "release");
    verify_abi_json(&project_dir, name, "release");

    let abi_v2 = std::fs::read(&abi_path).expect("read v2 abi");
    assert_eq!(
        abi_v1, abi_v2,
        "ABI bytes should be stable across a no-op rebuild"
    );
}

// `.sol` -> scaffold -> `cargo build` tests. The existing
// `scaffold_example`/`scaffold_new_contract` tests don't exercise the
// `init_from_solidity_file` path; per-type assertions live in the
// `scaffold::tests` unit tests on `solidity_to_rust_type`.

fn scaffold_from_sol_and_build(
    temp_dir: &TempDir,
    name: &str,
    api_style: &str,
    allocator: Option<&str>,
    sol_content: &str,
) -> PathBuf {
    let sol_path = temp_dir.path().join(format!("{name}.sol"));
    std::fs::write(&sol_path, sol_content).expect("write .sol fixture");

    let project_dir = temp_dir.path().join(name);
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("init")
        .arg("--init-type")
        .arg("new")
        .arg("--api-style")
        .arg(api_style)
        .arg("--name")
        .arg(name)
        .arg("--sol-file")
        .arg(&sol_path);
    if let Some(alloc) = allocator {
        cmd.arg("--allocator").arg(alloc);
    }
    cmd.assert().success();

    build_project(&project_dir, "debug");
    project_dir
}

fn sol_interface(name: &str, body: &str) -> String {
    format!(
        "// SPDX-License-Identifier: UNLICENSED\n\
         pragma solidity ^0.8.20;\n\
         interface {name} {{\n{body}\n}}\n"
    )
}

#[test]
fn scaffold_from_sol_macro_bump_dynamic_surface() {
    let temp_dir = TempDir::new().expect("temp dir");
    let sol = sol_interface(
        "MacroBumpSurface",
        "    function transfer(address to, uint256 amount) external returns (bool);\n\
         \x20   function setBytes(bytes calldata data) external returns (uint256);\n\
         \x20   function setName(string memory name) external returns (uint256);\n\
         \x20   function sum(uint256[3] calldata xs) external returns (uint256);\n\
         \x20   function bag(bytes[] calldata items) external returns (uint256);",
    );
    scaffold_from_sol_and_build(&temp_dir, "macro-bump-surface", "macro", Some("bump"), &sol);
}

#[test]
fn scaffold_from_sol_macro_no_alloc_static_surface() {
    let temp_dir = TempDir::new().expect("temp dir");
    let sol = sol_interface(
        "MacroNoAllocSurface",
        "    function transfer(address to, uint256 amount) external returns (bool);\n\
         \x20   function sum(uint256[3] calldata xs) external returns (uint256);\n\
         \x20   function double(uint256 x) external pure returns (uint256);",
    );
    scaffold_from_sol_and_build(
        &temp_dir,
        "macro-no-alloc-surface",
        "macro",
        Some("no-alloc"),
        &sol,
    );
}

#[test]
fn scaffold_from_sol_dsl_single_dynamic() {
    // Single-param dynamic input with no return is the only DSL shape that
    // survives the `StaticEncodedLen` safety net; other dynamic-in-DSL shapes
    // are rejected by the `scaffold_rejects_dsl_*` gate tests. The two
    // compound-type params also regression-test the `<T>::decode_at` wrap.
    let temp_dir = TempDir::new().expect("temp dir");
    let sol = sol_interface(
        "DslDynIface",
        "    function setBytes(bytes calldata data) external;\n\
         \x20   function setFixed(uint256[3] calldata xs) external;\n\
         \x20   function setDyn(uint256[] calldata xs) external;",
    );
    scaffold_from_sol_and_build(&temp_dir, "dsl-bytes-bump", "dsl", Some("bump"), &sol);
}

fn scaffold_init_with_dsl_dynamic(
    temp_dir: &TempDir,
    name: &str,
    sol: &str,
) -> std::process::Output {
    let sol_path = temp_dir.path().join(format!("{name}.sol"));
    std::fs::write(&sol_path, sol).expect("write .sol fixture");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("init")
        .arg("--init-type")
        .arg("new")
        .arg("--api-style")
        .arg("dsl")
        .arg("--allocator")
        .arg("bump")
        .arg("--name")
        .arg(name)
        .arg("--sol-file")
        .arg(&sol_path);
    cmd.output().expect("run cargo pvm-contract init")
}

#[test]
fn scaffold_rejects_dsl_dynamic_return() {
    let temp_dir = TempDir::new().expect("temp dir");
    let sol = sol_interface(
        "DynRet",
        "    function name() external view returns (bytes memory);",
    );
    let output = scaffold_init_with_dsl_dynamic(&temp_dir, "dyn-ret", &sol);

    assert!(
        !output.status.success(),
        "expected scaffold to fail with DSL + dynamic return"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("dynamic return types"),
        "expected dynamic-return error, got stderr: {stderr}"
    );
}

#[test]
fn scaffold_rejects_dsl_multi_param_with_dynamic() {
    let temp_dir = TempDir::new().expect("temp dir");
    let sol = sol_interface(
        "MultiDyn",
        "    function setMessage(bytes calldata data, uint256 nonce) external;",
    );
    let output = scaffold_init_with_dsl_dynamic(&temp_dir, "multi-dyn", &sol);

    assert!(
        !output.status.success(),
        "expected scaffold to fail with DSL + multi-param dynamic"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("multi-parameter signatures"),
        "expected multi-param-dynamic error, got stderr: {stderr}"
    );
}

#[test]
fn scaffold_rejects_dynamic_types_in_no_alloc_mode() {
    let temp_dir = TempDir::new().expect("temp dir");
    let sol_path = temp_dir.path().join("Dyn.sol");
    std::fs::write(
        &sol_path,
        sol_interface(
            "Dyn",
            "    function setBytes(bytes calldata data) external;",
        ),
    )
    .expect("write .sol fixture");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("cargo-pvm-contract"));
    cmd.current_dir(temp_dir.path())
        .env("CARGO_PVM_CONTRACT_PATH", workspace_path())
        .arg("pvm-contract")
        .arg("init")
        .arg("--init-type")
        .arg("new")
        .arg("--api-style")
        .arg("macro")
        .arg("--allocator")
        .arg("no-alloc")
        .arg("--name")
        .arg("dyn-no-alloc")
        .arg("--sol-file")
        .arg(&sol_path);

    let output = cmd.output().expect("run cargo pvm-contract init");
    assert!(
        !output.status.success(),
        "expected scaffold to fail with no-alloc + dynamic types"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("require an allocator"),
        "expected actionable allocator error, got stderr: {stderr}"
    );
}
