# cargo-pvm-contract

A cargo subcommand to build Rust contracts to PolkaVM bytecode.

## Installation

```bash
cargo install --path .
```

Alternatively, you can run it directly from the target directory:

```bash
./target/release/cargo-pvm-contract pvm-contract [COMMAND]
```

## Usage

Once installed, you can use it as a cargo subcommand:

```bash
cargo pvm-contract <COMMAND>
```

### Commands

#### `init` - Initialize a new contract project

Create a new contract project from a template:

```bash
cargo pvm-contract init <CONTRACT_NAME> [OPTIONS]
```

Options:
- `<CONTRACT_NAME>` - Name of the contract project (required)
- `-t, --template <TEMPLATE>` - Template to use (defaults to `pico-alloc`)

Examples:

Initialize with default pico-alloc template:

```bash
cargo pvm-contract init my-token
cd my-token
cargo pvm-contract build
```

Initialize with a specific template:

```bash
cargo pvm-contract init my-token --template pico-alloc
```

#### `build` - Build a contract to PolkaVM bytecode

Build a contract binary to PolkaVM bytecode:

```bash
cargo pvm-contract build [BIN_NAME]
```

Options:
- `[BIN_NAME]` - Name of the binary to build (optional, defaults to first binary in Cargo.toml)
- `-o, --output <PATH>` - Output path for the PolkaVM bytecode (defaults to `./<bin_name>.polkavm`)

Examples:

Build the first binary defined in Cargo.toml:

```bash
cargo pvm-contract build
```

## Templates

The tool includes contract templates to help you get started quickly. Templates are located in the `templates/` directory.

