use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

/// Each test gets its own revive-dev-node on a unique port.
/// `Drop` kills the process when the test ends.
static NEXT_PORT: AtomicU16 = AtomicU16::new(29545);

/// The `revive-dev-node` binary from polkadot-sdk.
///
/// Runs in `--dev` mode with pre-funded sr25519 dev accounts (Alice, Bob, etc.).
/// Set `REVIVE_DEV_NODE` env var to override the binary path.
pub struct SubstrateDevNode {
    ws_url: String,
    child: Child,
}

impl SubstrateDevNode {
    pub fn start() -> Self {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        let binary =
            std::env::var("REVIVE_DEV_NODE").unwrap_or_else(|_| "revive-dev-node".to_string());

        let mut child = Command::new(&binary)
            .args([
                "--dev",
                "--rpc-port",
                &port.to_string(),
                "--no-prometheus",
                "--log",
                "error,sc_rpc_server=info",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to start revive-dev-node (binary: {binary}): {e}\n\
                     Build it from polkadot-sdk:\n  \
                     cargo build -p revive-dev-node --release"
                )
            });

        let ws_url = format!("ws://127.0.0.1:{port}");

        // Wait for the node to start accepting RPC connections by watching
        // stderr for the "Running JSON-RPC server" message.
        let stderr = child.stderr.take().expect("stderr is piped");
        let reader = BufReader::new(stderr);
        let start = Instant::now();
        let mut ready = false;

        for line in reader.lines() {
            if start.elapsed() > Duration::from_secs(60) {
                break;
            }
            match line {
                Ok(l) => {
                    if l.contains("Running JSON-RPC server") {
                        ready = true;
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        if !ready {
            let _ = child.kill();
            panic!("revive-dev-node failed to start on port {port} within 60 seconds");
        }

        Self { ws_url, child }
    }

    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }
}

impl Drop for SubstrateDevNode {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
