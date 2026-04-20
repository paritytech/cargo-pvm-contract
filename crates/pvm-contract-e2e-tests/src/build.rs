use assert_cmd::cargo::cargo_bin;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};

static BUILT: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn contract(name: &str) -> Contract {
    Contract {
        dir: workspace_root().join(format!("examples/{name}")),
    }
}

pub struct Contract {
    dir: PathBuf,
}

impl Contract {
    /// Build the contract project (release). Only runs once per unique project path.
    pub fn build(&self) {
        let mut built = BUILT.lock().unwrap();
        if built.contains(&self.dir) {
            return;
        }

        let name = self.dir.file_name().unwrap().to_str().unwrap();
        assert!(
            self.dir.join("Cargo.toml").exists(),
            "{name} project not found at {}",
            self.dir.display()
        );

        let status = Command::new(cargo_bin("cargo-pvm-contract"))
            .current_dir(&self.dir)
            .args(["pvm-contract", "build"])
            .status()
            .unwrap_or_else(|_| panic!("Failed to run cargo pvm-contract build on {name}"));

        assert!(
            status.success(),
            "cargo pvm-contract build failed for {name}"
        );

        built.insert(self.dir.clone());
    }

    pub fn target(&self) -> PathBuf {
        self.dir.join("target")
    }

    pub fn polkavm_binary(&self, binary_name: &str, profile: &str) -> PathBuf {
        let path = self
            .target()
            .join(profile)
            .join(format!("{binary_name}.polkavm"));
        assert!(
            path.exists(),
            "PolkaVM binary not found: {}",
            path.display()
        );
        path
    }

    pub fn abi_json_path(&self, binary_name: &str, profile: &str) -> PathBuf {
        let path = self
            .target()
            .join(profile)
            .join(format!("{binary_name}.abi.json"));
        assert!(path.exists(), "ABI JSON not found: {}", path.display());
        path
    }

    pub fn bytecode_hex(&self, binary_name: &str, profile: &str) -> String {
        let path = self.polkavm_binary(binary_name, profile);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|_| panic!("Failed to read {}", path.display()));
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        format!("0x{hex}")
    }
}
