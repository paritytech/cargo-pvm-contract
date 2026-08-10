//! Writes the golden fixtures the viem round-trip suite loads.
//!
//! Two kinds of output land in `ts/viem-roundtrip/fixtures/`:
//!
//! * `*.abi.json` — rendered through the same `render_abi_json` the real build
//!   calls, so the TypeScript suite reads the same JSON a user's
//!   `target/{profile}/{bin}.abi.json` would contain (plus a trailing newline,
//!   added here for diff hygiene). No riscv build, no PolkaVM link.
//! * `vectors.json` — the encoded corpus from [`pvm_viem_roundtrip::corpus`].
//!
//! Both are checked in. CI reruns this and fails on any diff, so an ABI or
//! encoding change has to be reviewed rather than silently absorbed. To re-bless
//! a legitimate change, run this and commit the result.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// A contract binary whose emitted ABI becomes a fixture.
struct Target {
    /// Contract project directory, relative to the repo root.
    project: &'static str,
    /// Binary name within that project.
    bin: &'static str,
    /// Fixture file stem; also the export name in the generated `abis.ts`.
    name: &'static str,
}

/// The `.sol`-backed binaries under `examples/test-contracts`.
///
/// These cost nothing to render: the builder derives their ABI by parsing the
/// `.sol` interface, so no compilation of any kind is involved.
const SOL_TARGETS: &[Target] = &[
    Target {
        project: "examples/test-contracts",
        bin: "flipper",
        name: "flipper",
    },
    Target {
        project: "examples/test-contracts",
        bin: "multi-method",
        name: "multi-method",
    },
    Target {
        project: "examples/test-contracts",
        bin: "return-values",
        name: "return-values",
    },
    Target {
        project: "examples/test-contracts",
        bin: "dynamic-types",
        name: "dynamic-types",
    },
    Target {
        project: "examples/test-contracts",
        bin: "composite-types",
        name: "composite-types",
    },
    Target {
        project: "examples/test-contracts",
        bin: "storage-types",
        name: "storage-types",
    },
    Target {
        project: "examples/test-contracts",
        bin: "point_adder",
        name: "point-adder",
    },
    Target {
        project: "examples/test-contracts",
        bin: "constructor-args",
        name: "constructor-args",
    },
    Target {
        project: "examples/test-contracts",
        bin: "events",
        name: "events",
    },
    Target {
        project: "examples/test-contracts",
        bin: "error-handling",
        name: "error-handling",
    },
    Target {
        project: "examples/test-contracts",
        bin: "error_caller",
        name: "error-caller",
    },
    Target {
        project: "examples/test-contracts",
        bin: "payable",
        name: "payable",
    },
    Target {
        project: "examples/test-contracts",
        bin: "receive",
        name: "receive",
    },
    Target {
        project: "examples/test-contracts",
        bin: "caller-check",
        name: "caller-check",
    },
    Target {
        project: "examples/test-contracts",
        bin: "flipper_call",
        name: "flipper-call",
    },
    Target {
        project: "examples/test-contracts",
        bin: "flipper_delegate",
        name: "flipper-delegate",
    },
    Target {
        project: "examples/test-contracts",
        bin: "point_adder_call",
        name: "point-adder-call",
    },
];

/// Binaries whose ABI has to come from the Rust side.
///
/// These are the expensive ones: the builder compiles and runs the binary for
/// the host with `-Zbuild-std` to read its `__abi_json()` output. The in-crate
/// `abi-surface` contract covers the Rust emitter far more thoroughly for free,
/// so only one target is listed here — the one that also produces the
/// `{"abi":…,"storageLayout":…}` container shape, which nothing else can.
const RUST_TARGETS: &[Target] = &[Target {
    project: "examples/example-mytoken",
    bin: "example-mytoken-macro-storage",
    name: "mytoken-storage",
}];

/// Standalone Solidity interfaces under this crate's `sol/` directory.
///
/// Nothing implements them; they exist to drive `type_to_abi_param` over every
/// branch of the `.sol` type mapping, which the interfaces in
/// `examples/test-contracts` only sparsely cover. Keeping them here rather than
/// in a contract project means no Rust implementation and no riscv build.
const SOL_FILES: &[(&str, &str)] = &[
    ("SolTypeSurface.sol", "sol-type-surface"),
    ("SolReferenceTypes.sol", "sol-reference-types"),
];

fn main() -> Result<()> {
    let repo_root = repo_root()?;
    let fixtures = repo_root.join("ts/viem-roundtrip/fixtures");
    let abi_dir = fixtures.join("abi");
    std::fs::create_dir_all(&abi_dir)
        .with_context(|| format!("Failed to create {}", abi_dir.display()))?;

    let target_root = repo_root.join("target");
    let mut written = Vec::new();

    for target in SOL_TARGETS.iter().chain(RUST_TARGETS) {
        let manifest_dir = repo_root.join(target.project);
        let json = cargo_pvm_contract_builder::render_abi_json(
            &manifest_dir,
            target.bin,
            Some(&target_root),
            None,
        )
        .with_context(|| format!("Failed to render ABI for {}", target.bin))?
        .with_context(|| {
            format!(
                "{} has no #[contract] module, so it cannot be an ABI fixture",
                target.bin
            )
        })?;

        write_if_changed(&abi_dir.join(format!("{}.abi.json", target.name)), &json)?;
        written.push(target.name);
    }

    let sol_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sol");
    for (file, name) in SOL_FILES {
        let json = cargo_pvm_contract_builder::render_abi_from_sol(&sol_dir.join(file))
            .with_context(|| format!("Failed to render ABI from {file}"))?
            .with_context(|| format!("{file} declares nothing that maps to an ABI item"))?;
        write_if_changed(&abi_dir.join(format!("{name}.abi.json")), &json)?;
        written.push(name);
    }

    // The in-crate `abi-surface` contract goes through the macro's own
    // `__abi_json()` accessor rather than the builder, because it has no
    // contract project of its own. This is the same string the builder would
    // read from the abi-gen binary's stdout.
    let surface = pvm_viem_roundtrip::surface::abi_surface::__abi_json();
    write_if_changed(&abi_dir.join("abi-surface.abi.json"), &surface)?;
    written.push("abi-surface");

    let vectors = pvm_viem_roundtrip::corpus::build();
    let vectors_json =
        serde_json::to_string_pretty(&vectors).context("Failed to serialize the fixture corpus")?;
    write_if_changed(&fixtures.join("vectors.json"), &vectors_json)?;

    // Every corpus contract must name an ABI file that actually exists,
    // otherwise the TypeScript loader fails at import time with a much less
    // useful message than this one.
    for contract in &vectors.contracts {
        let path = fixtures.join(&contract.abi_file);
        if !path.exists() {
            bail!(
                "corpus contract `{}` references {}, which was not generated",
                contract.name,
                contract.abi_file
            );
        }
    }

    eprintln!(
        "Wrote {} ABI fixtures and {} corpus contracts to {}",
        written.len(),
        vectors.contracts.len(),
        fixtures.display()
    );
    Ok(())
}

/// Write `contents` only when it differs from what is already on disk, so an
/// unchanged run leaves mtimes alone and `git diff` stays the single source of
/// truth for "did anything change".
fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
    // Trailing newline: keeps the files POSIX-clean and diff-friendly.
    let contents = format!("{}\n", contents.trim_end());
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == contents
    {
        return Ok(());
    }
    std::fs::write(path, &contents)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Walk up from this crate's manifest directory to the workspace root.
fn repo_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").exists() && dir.join("crates").is_dir())
        .map(Path::to_path_buf)
        .context("Could not locate the workspace root above CARGO_MANIFEST_DIR")
}
