//! Shared solc invocation for the differential test modules.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `solc --standard-json` over a single source `C.sol`, requesting the
/// given `outputs` (e.g. `["storageLayout"]` or `["evm.deployedBytecode.object"]`),
/// and return the parsed output JSON. Panics on a spawn failure, a non-zero
/// exit, or any solc error diagnostic.
pub fn run_solc(source: &str, outputs: &[&str]) -> serde_json::Value {
    let input = serde_json::json!({
        "language": "Solidity",
        "sources": { "C.sol": { "content": source } },
        "settings": {
            "outputSelection": { "*": { "*": outputs } },
            "optimizer": { "enabled": false },
            // Pin the EVM version so bytecode is deterministic across solc
            // releases and targets a hardfork `revm` supports.
            "evmVersion": "cancun"
        }
    });

    let mut child = Command::new("solc")
        .arg("--standard-json")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn solc — is it installed and on PATH?");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait for solc");
    assert!(
        out.status.success(),
        "solc exited non-zero:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("solc output parses as json");

    // Standard-json reports compile errors in an `errors` array with exit 0.
    if let Some(errors) = parsed["errors"].as_array() {
        let fatal: Vec<&str> = errors
            .iter()
            .filter(|e| e["severity"].as_str() == Some("error"))
            .filter_map(|e| e["formattedMessage"].as_str())
            .collect();
        assert!(
            fatal.is_empty(),
            "solc reported errors:\n{}",
            fatal.join("\n")
        );
    }

    parsed
}
