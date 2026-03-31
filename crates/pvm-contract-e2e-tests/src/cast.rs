use std::process::{Command, Output};

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

        let output = Command::new("cast")
            .args([
                "send",
                "--rpc-url",
                &self.rpc_url,
                "--private-key",
                private_key,
                "--gas-limit",
                "9999999999999",
                "--json",
                "--create",
                &bytecode,
            ])
            .output()
            .expect("cast send --create failed to execute");

        assert!(
            output.status.success(),
            "cast deploy failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: serde_json::Value =
            serde_json::from_str(&stdout).expect("Failed to parse cast deploy output as JSON");

        let status = json["status"].as_str().unwrap_or("0x0");
        assert_eq!(
            status,
            "0x1",
            "Deploy transaction reverted: {}",
            json.get("revertReason")
                .and_then(|r| r.as_str())
                .unwrap_or("unknown")
        );

        json["contractAddress"]
            .as_str()
            .expect("No contractAddress in cast output")
            .to_string()
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
            "--json",
        ]);

        let output = cmd.output().expect("cast send failed to execute");
        assert!(
            output.status.success(),
            "cast send '{sig}' failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .expect("Failed to parse cast send output as JSON");
        json["transactionHash"]
            .as_str()
            .expect("No transactionHash in cast output")
            .to_string()
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
