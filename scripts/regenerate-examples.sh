#!/bin/bash
set -euo pipefail

# Get the repository root (parent of scripts directory)
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Build the CLI binary once
echo "Building cargo-pvm-contract CLI..."
cargo build -p cargo-pvm-contract

BINARY="$REPO_ROOT/target/debug/cargo-pvm-contract"

# Export the environment variable for relative path resolution
export CARGO_PVM_CONTRACT_PATH=../..

# Change to examples directory
cd "$REPO_ROOT/examples"

# Function to generate a project variant
generate_project() {
	local name="$1"
	shift

	# Remove existing directory if it exists
	rm -rf "$name"

	# Run the CLI command
	"$BINARY" pvm-contract "$@" --name "$name"
}

echo "Generating all 10 project variants..."

# Example-based scaffolds (--init-type example)
generate_project "example-fibonacci-alloc" \
	--init-type example \
	--example Fibonacci \
	--memory-model alloc-with-alloy

generate_project "example-fibonacci-no-alloc" \
	--init-type example \
	--example Fibonacci \
	--memory-model no-alloc

generate_project "example-mytoken-alloc" \
	--init-type example \
	--example MyToken \
	--memory-model alloc-with-alloy

generate_project "example-mytoken-no-alloc" \
	--init-type example \
	--example MyToken \
	--memory-model no-alloc

# Blank new contract (--init-type new, no --sol-file)
generate_project "new-blank-alloc" \
	--init-type new \
	--memory-model alloc-with-alloy

generate_project "new-blank-no-alloc" \
	--init-type new \
	--memory-model no-alloc

# New from Solidity file (--init-type new --sol-file)
generate_project "new-from-sol-fibonacci-alloc" \
	--init-type new \
	--memory-model alloc-with-alloy \
	--sol-file "$REPO_ROOT/crates/cargo-pvm-contract/templates/examples/fibonacci/Fibonacci.sol"

generate_project "new-from-sol-fibonacci-no-alloc" \
	--init-type new \
	--memory-model no-alloc \
	--sol-file "$REPO_ROOT/crates/cargo-pvm-contract/templates/examples/fibonacci/Fibonacci.sol"

generate_project "new-from-sol-mytoken-alloc" \
	--init-type new \
	--memory-model alloc-with-alloy \
	--sol-file "$REPO_ROOT/crates/cargo-pvm-contract/templates/examples/mytoken/MyToken.sol"

generate_project "new-from-sol-mytoken-no-alloc" \
	--init-type new \
	--memory-model no-alloc \
	--sol-file "$REPO_ROOT/crates/cargo-pvm-contract/templates/examples/mytoken/MyToken.sol"

echo "All 10 project variants generated successfully!"
