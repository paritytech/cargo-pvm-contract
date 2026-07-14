use std::process::{Command, Output};
use std::time::{Duration, Instant};

pub const DEFAULT_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub const DEFAULT_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

pub struct CastClient {
    pub rpc_url: String,
}

impl CastClient {
    pub fn new(rpc_url: &str) -> Self {
        Self {
            rpc_url: rpc_url.to_string(),
        }
    }

    /// Broadcast a `cast send` transaction exactly once and return its mined
    /// receipt as JSON.
    ///
    /// We do NOT let `cast send` await the receipt: alloy's watcher gives up with
    /// a fatal `NullResp` once it sees the tx's block but `eth_getTransactionReceipt`
    /// is still `null` — which happens on anvil-polkadot under load, because the
    /// head advances before receipts are queryable (see issue #116). A `null`
    /// receipt only ever means "pending", so we poll it ourselves.
    ///
    /// `tx_args` is everything that identifies the call (`--private-key`,
    /// `--gas-limit`, `--value`, and either `--create <code>` or
    /// `<to> <sig> <args...>`); `--rpc-url` and `--async` are prepended here so a
    /// trailing `--create <code>` stays last, which is where cast expects it.
    fn broadcast_and_get_receipt(&self, tx_args: &[&str]) -> Result<serde_json::Value, String> {
        let send = Command::new("cast")
            .arg("send")
            .args(["--rpc-url", &self.rpc_url, "--async"])
            .args(tx_args)
            .output()
            .expect("cast send --async failed to execute");
        if !send.status.success() {
            return Err(String::from_utf8_lossy(&send.stderr).to_string());
        }
        // `cast send --async` prints just the tx hash.
        let tx_hash = String::from_utf8_lossy(&send.stdout).trim().to_string();

        let start = Instant::now();
        loop {
            // 10s is >100x the worst receipt latency observed under CPU saturation
            if start.elapsed() > Duration::from_secs(10) {
                return Err(format!("timed out waiting for receipt of {tx_hash}"));
            }
            let out = Command::new("cast")
                .args([
                    "rpc",
                    "eth_getTransactionReceipt",
                    &tx_hash,
                    "--rpc-url",
                    &self.rpc_url,
                ])
                .output()
                .expect("cast rpc eth_getTransactionReceipt failed to execute");
            if out.status.success() {
                let raw = String::from_utf8_lossy(&out.stdout);
                let raw = raw.trim();
                if !raw.is_empty() && raw != "null" {
                    return serde_json::from_str(raw)
                        .map_err(|e| format!("failed to parse receipt JSON: {e}"));
                }
            }
            // 100ms is the optimal estimated polling interval for `cast send --async`
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Return a mined receipt's transaction hash, asserting the tx succeeded
    /// (`status == 0x1`). Used by the write-send helpers, which represent
    /// "send and expect success": a reverting call that slips past cast's gas
    /// estimation would otherwise mine with `status == 0x0` and be silently
    /// treated as success. `label` identifies the call in the panic message.
    fn tx_hash_expect_success(receipt: &serde_json::Value, label: &str) -> String {
        let tx_hash = receipt["transactionHash"]
            .as_str()
            .expect("No transactionHash in receipt");
        let status = receipt["status"].as_str().unwrap_or("0x0");
        assert_eq!(status, "0x1", "{label} reverted (tx {tx_hash})");
        tx_hash.to_string()
    }

    /// Faithful revert reason for a mined-but-reverted tx, from
    /// `debug_traceTransaction` with the callTracer. Runs only on the failure path.
    fn revert_reason(&self, tx_hash: &str) -> String {
        let Ok(out) = Command::new("cast")
            .args([
                "rpc",
                "debug_traceTransaction",
                tx_hash,
                r#"{"tracer":"callTracer"}"#,
                "--rpc-url",
                &self.rpc_url,
            ])
            .output()
        else {
            return "unknown".to_string();
        };
        let Ok(trace) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
            return "unknown".to_string();
        };
        let error = trace["error"].as_str().unwrap_or("");
        let output = trace["output"]
            .as_str()
            .filter(|o| *o != "0x")
            .unwrap_or("");
        match (error, output) {
            ("", "") => "unknown".to_string(),
            (error, "") => error.to_string(),
            ("", output) => output.to_string(),
            (error, output) => format!("{error} ({output})"),
        }
    }

    /// Deploy a contract and return the contract address.
    /// If `constructor_sig` is non-empty, ABI-encodes `args` and appends them to the bytecode.
    pub fn deploy(
        &self,
        bytecode_hex: &str,
        constructor_sig: &str,
        args: &[&str],
        private_key: &str,
    ) -> String {
        let bytecode = if !constructor_sig.is_empty() {
            let mut cmd = Command::new("cast");
            cmd.args(["abi-encode", constructor_sig]);
            for arg in args {
                cmd.arg(arg);
            }
            let output = cmd.output().expect("cast abi-encode failed to execute");
            assert!(
                output.status.success(),
                "cast abi-encode failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let encoded_args = String::from_utf8(output.stdout)
                .unwrap()
                .trim()
                .trim_start_matches("0x")
                .to_string();
            format!("{bytecode_hex}{encoded_args}")
        } else {
            bytecode_hex.to_string()
        };

        let receipt = self
            .broadcast_and_get_receipt(&[
                "--private-key",
                private_key,
                "--gas-limit",
                "9999999999999",
                "--create",
                &bytecode,
            ])
            .unwrap_or_else(|e| panic!("cast deploy failed: {e}"));

        let status = receipt["status"].as_str().unwrap_or("0x0");
        let tx_hash = receipt["transactionHash"].as_str().unwrap_or("?");
        assert_eq!(
            status,
            "0x1",
            "Deploy transaction reverted (tx {tx_hash}): {}",
            self.revert_reason(tx_hash)
        );

        receipt["contractAddress"]
            .as_str()
            .expect("No contractAddress in receipt")
            .to_string()
    }

    /// Deploy a contract with a value transfer and return the contract address on success.
    pub fn deploy_with_value(
        &self,
        bytecode_hex: &str,
        constructor_sig: &str,
        args: &[&str],
        private_key: &str,
        value: &str,
    ) -> Result<String, String> {
        let bytecode = if !constructor_sig.is_empty() {
            let mut cmd = Command::new("cast");
            cmd.args(["abi-encode", constructor_sig]);
            for arg in args {
                cmd.arg(arg);
            }
            let output = cmd.output().expect("cast abi-encode failed to execute");
            assert!(
                output.status.success(),
                "cast abi-encode failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let encoded_args = String::from_utf8(output.stdout)
                .unwrap()
                .trim()
                .trim_start_matches("0x")
                .to_string();
            format!("{bytecode_hex}{encoded_args}")
        } else {
            bytecode_hex.to_string()
        };

        let receipt = self.broadcast_and_get_receipt(&[
            "--private-key",
            private_key,
            "--gas-limit",
            "9999999999999",
            "--value",
            value,
            "--create",
            &bytecode,
        ])?;

        let status = receipt["status"].as_str().unwrap_or("0x0");
        if status != "0x1" {
            let tx_hash = receipt["transactionHash"].as_str().unwrap_or("?");
            return Err(format!(
                "Deploy transaction reverted (tx {tx_hash}): {}",
                self.revert_reason(tx_hash)
            ));
        }

        Ok(receipt["contractAddress"]
            .as_str()
            .expect("No contractAddress in receipt")
            .to_string())
    }

    /// Call a read-only function and return the raw output string.
    pub fn call(&self, contract: &str, sig: &str, args: &[&str]) -> String {
        let mut cmd = Command::new("cast");
        cmd.args(["call", contract, sig]);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.args(["--rpc-url", &self.rpc_url, "--from", DEFAULT_ADDRESS]);

        let output = cmd.output().expect("cast call failed to execute");
        assert!(
            output.status.success(),
            "cast call '{sig}' failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let raw = String::from_utf8(output.stdout).unwrap().trim().to_string();
        // cast annotates large numbers like "999999 [9.999e5]" — strip the annotation
        match raw.find(" [") {
            Some(pos) => raw[..pos].to_string(),
            None => raw,
        }
    }

    /// Send a write transaction. Returns the transaction hash.
    pub fn send(&self, contract: &str, sig: &str, args: &[&str], private_key: &str) -> String {
        let mut tx_args: Vec<&str> = vec![contract, sig];
        tx_args.extend_from_slice(args);
        tx_args.extend_from_slice(&["--private-key", private_key]);

        let receipt = self
            .broadcast_and_get_receipt(&tx_args)
            .unwrap_or_else(|e| panic!("cast send '{sig}' failed: {e}"));
        Self::tx_hash_expect_success(&receipt, &format!("cast send '{sig}'"))
    }

    /// Send a write transaction with a value transfer. Returns the transaction hash.
    pub fn send_with_value(
        &self,
        contract: &str,
        sig: &str,
        args: &[&str],
        private_key: &str,
        value: &str,
    ) -> String {
        let mut tx_args: Vec<&str> = vec![contract, sig];
        tx_args.extend_from_slice(args);
        tx_args.extend_from_slice(&["--private-key", private_key, "--value", value]);

        let receipt = self
            .broadcast_and_get_receipt(&tx_args)
            .unwrap_or_else(|e| panic!("cast send '{sig}' failed: {e}"));
        Self::tx_hash_expect_success(&receipt, &format!("cast send '{sig}'"))
    }

    /// Send a plain ether transfer (empty calldata) to a contract address.
    /// Targets the contract's `receive` (or payable `fallback`) handler.
    pub fn send_value_only(&self, contract: &str, private_key: &str, value: &str) -> String {
        let tx_args: Vec<&str> = vec![contract, "--private-key", private_key, "--value", value];

        let receipt = self
            .broadcast_and_get_receipt(&tx_args)
            .unwrap_or_else(|e| panic!("cast send (value-only) failed: {e}"));
        Self::tx_hash_expect_success(&receipt, "cast send (value-only)")
    }

    /// Send a write transaction with a value transfer, expect it to revert. Returns raw output.
    pub fn send_with_value_expect_revert(
        &self,
        contract: &str,
        sig: &str,
        args: &[&str],
        private_key: &str,
        value: &str,
    ) -> Output {
        let mut cmd = Command::new("cast");
        cmd.args(["send", contract, sig]);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.args([
            "--rpc-url",
            &self.rpc_url,
            "--private-key",
            private_key,
            "--value",
            value,
        ]);

        cmd.output().expect("cast send failed to execute")
    }

    /// Send a write transaction, expect it to revert. Returns raw output.
    pub fn send_expect_revert(
        &self,
        contract: &str,
        sig: &str,
        args: &[&str],
        private_key: &str,
    ) -> Output {
        let mut cmd = Command::new("cast");
        cmd.args(["send", contract, sig]);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.args(["--rpc-url", &self.rpc_url, "--private-key", private_key]);

        cmd.output().expect("cast send failed to execute")
    }

    /// The 4-byte selector (lowercase hex, no `0x`) for a function or error
    /// signature, computed as `keccak256(sig)[..4]` via `cast keccak`.
    pub fn selector(&self, sig: &str) -> String {
        let output = Command::new("cast")
            .args(["keccak", sig])
            .output()
            .expect("cast keccak failed to execute");
        assert!(
            output.status.success(),
            "cast keccak '{sig}' failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let hash = String::from_utf8(output.stdout).unwrap();
        let hash = hash.trim().trim_start_matches("0x");
        hash[..8].to_lowercase()
    }

    /// Get logs for a specific event signature.
    pub fn logs(&self, contract: &str, event_sig: &str) -> String {
        let output = Command::new("cast")
            .args([
                "logs",
                "--from-block",
                "0",
                "--address",
                contract,
                event_sig,
                "--rpc-url",
                &self.rpc_url,
            ])
            .output()
            .expect("cast logs failed to execute");

        String::from_utf8(output.stdout).unwrap()
    }
}
