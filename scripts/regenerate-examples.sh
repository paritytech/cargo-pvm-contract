#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Building cargo-pvm-contract CLI..."
cargo build -p cargo-pvm-contract

BINARY="$REPO_ROOT/target/debug/cargo-pvm-contract"

export CARGO_PVM_CONTRACT_PATH=../..

cd "$REPO_ROOT/examples"

generate_project() {
	local name="$1"
	shift

	rm -rf "$name"
	"$BINARY" pvm-contract "$@" --name "$name"
}

echo "Cleaning existing generated example projects..."
for dir in */; do
	[ -d "$dir" ] || continue
	rm -rf "$dir"
done

echo "Generating base mytoken examples from CLI..."
generate_project "example-mytoken" \
	--init-type example \
	--example MyToken \
	--memory-model alloc-with-alloy

generate_project ".tmp-example-mytoken-no-alloc" \
	--init-type example \
	--example MyToken \
	--memory-model no-alloc

cp "example-mytoken/src/my-token.rs" "example-mytoken/src/example-mytoken-macro-alloc.rs"
cp ".tmp-example-mytoken-no-alloc/src/my-token.rs" "example-mytoken/src/example-mytoken-macro-no-alloc.rs"
cp "$REPO_ROOT/crates/pvm-contract-builder-dsl/contracts/mytoken_builder.rs" "example-mytoken/src/example-mytoken-dsl-no-alloc.rs"
cp "$REPO_ROOT/crates/cargo-pvm-contract/templates/examples/mytoken/mytoken_alloy.rs" "example-mytoken/src/example-mytoken-alloy-alloc.rs"
perl -0pi -e 's/use ruint::aliases::U256;\n/use ruint::aliases::U256;\n\n#[global_allocator]\nstatic mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<1024>>> = {\n    static mut ARRAY: picoalloc::Array<1024> = picoalloc::Array([0u8; 1024]);\n\n    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {\n        picoalloc::ArrayPointer::new(&raw mut ARRAY)\n    }))\n};\n/' "example-mytoken/src/example-mytoken-macro-no-alloc.rs"
perl -0pi -e 's/^#!\[cfg\(any\(target_arch = "riscv32", target_arch = "riscv64"\)\)\]\n//' "example-mytoken/src/example-mytoken-dsl-no-alloc.rs"
perl -0pi -e 's/use pallet_revive_uapi::StorageFlags;/use pvm_contract_builder_dsl::pallet_revive_uapi::StorageFlags;/' "example-mytoken/src/example-mytoken-dsl-no-alloc.rs"
perl -0pi -e 's/use pallet_revive_uapi::\{HostFn as _, HostFnImpl, ReturnFlags\};/use pvm_contract_builder_dsl::pallet_revive_uapi::{HostFn as _, HostFnImpl, ReturnFlags};/' "example-mytoken/src/example-mytoken-dsl-no-alloc.rs"
perl -0pi -e 's/use pallet_revive_uapi::HostFnImpl as api;/use pvm_contract_builder_dsl::pallet_revive_uapi::HostFnImpl as api;/' "example-mytoken/src/example-mytoken-dsl-no-alloc.rs"
perl -0pi -e 's/use ruint::aliases::U256;\n/use ruint::aliases::U256;\n\n#[global_allocator]\nstatic mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<1024>>> = {\n    static mut ARRAY: picoalloc::Array<1024> = picoalloc::Array([0u8; 1024]);\n\n    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {\n        picoalloc::ArrayPointer::new(&raw mut ARRAY)\n    }))\n};\n/' "example-mytoken/src/example-mytoken-dsl-no-alloc.rs"
perl -0pi -e 's/abi_decode\(input, true\)/abi_decode(input)/g' "example-mytoken/src/example-mytoken-alloy-alloc.rs"
rm -f "example-mytoken/src/my-token.rs"
rm -rf ".tmp-example-mytoken-no-alloc"

cat >"example-mytoken/Cargo.toml" <<'EOF'
[workspace]

[package]
name = "example-mytoken"
version = "0.1.0"
edition = "2024"
rust-version = "1.92"
build = "build.rs"

[[bin]]
name = "example-mytoken-macro-alloc"
path = "src/example-mytoken-macro-alloc.rs"

[[bin]]
name = "example-mytoken-macro-no-alloc"
path = "src/example-mytoken-macro-no-alloc.rs"

[[bin]]
name = "example-mytoken-dsl-no-alloc"
path = "src/example-mytoken-dsl-no-alloc.rs"

[[bin]]
name = "example-mytoken-alloy-alloc"
path = "src/example-mytoken-alloy-alloc.rs"

[dependencies]
pvm-contract-macros = { path = "../../crates/pvm-contract-macros" }
pvm-contract-types = { path = "../../crates/pvm-contract-types" }
pvm-contract-builder-dsl = { path = "../../crates/pvm-contract-builder-dsl" }
alloy-core = { version = "1.5", default-features = false, features = ["sol-types"] }
pallet-revive-uapi = { version = "0.9", default-features = false }
polkavm-derive = { version = "0.31.0" }
ruint = { version = "1.17", default-features = false }
picoalloc = { version = "5", default-features = false }

[build-dependencies]
cargo-pvm-contract-builder = { path = "../../crates/cargo-pvm-contract-builder" }

[profile.release]
codegen-units = 1
lto = true
opt-level = "z"
panic = "abort"
overflow-checks = false
EOF

echo "Generated mytoken example with 4 variants in examples/example-mytoken"
