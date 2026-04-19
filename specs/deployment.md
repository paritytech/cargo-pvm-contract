# Deploying and Interacting with Contracts

After building, you get two artifacts:

```text
target/release/my_contract.polkavm    — deployable bytecode
target/release/my_contract.abi.json   — Ethereum-compatible ABI JSON
```

You can deploy and interact with contracts using **two paths**.

## Substrate Path (recommended)

Use `cargo pvm-contract` subcommands for native Substrate RPC. Contracts still use Ethereum ABI, but you do not need Ethereum RPC tooling.

See [cli.md](cli.md) for the full `cargo pvm-contract` CLI reference.

## Ethereum Compatibility Path

Use this when you specifically want Ethereum-style RPC workflows via Foundry tooling.

### Install foundry-polkadot

[foundry-polkadot](https://github.com/paritytech/foundry-polkadot) is a Polkadot-adapted fork of Foundry that provides `cast`, and `anvil-polkadot`:

```bash
curl -L https://raw.githubusercontent.com/paritytech/foundry-polkadot/refs/heads/master/foundryup/install | bash
foundryup-polkadot
```

This gives you:

- **`cast`** — deploy contracts and send transactions
- **`anvil-polkadot`** — local Substrate node with Ethereum-compatible RPC

### Local Testing with anvil-polkadot

`anvil-polkadot` is a Substrate-based local node with an Ethereum-compatible RPC API. It runs pallet-revive locally so you can test contracts without a remote testnet:

```bash
# Start a local node (listens on http://127.0.0.1:8545)
anvil-polkadot
```

Then deploy and interact against it:

```bash
BYTECODE=0x$(xxd -p target/release/my_contract.polkavm | tr -d '\n')

cast send \
  --rpc-url http://127.0.0.1:8545 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --gas-limit 9999999999999 \
  --create $BYTECODE
```

### Deploy a Contract

Convert the `.polkavm` bytecode to hex and deploy with `cast`:

```bash
RPC=http://127.0.0.1:8545

# Convert binary to hex
BYTECODE=0x$(xxd -p target/release/my_contract.polkavm | tr -d '\n')

# Deploy (sends a create transaction with the bytecode)
cast send \
  --rpc-url $RPC \
  --private-key $PRIVATE_KEY \
  --gas-limit 9999999999999 \
  --create $BYTECODE
```

The transaction receipt contains the deployed contract address.

To deploy with constructor arguments, append ABI-encoded args to the bytecode:

```bash
# Example: constructor that takes an initial supply (uint256)
CONSTRUCTOR_ARGS=$(cast abi-encode "constructor(uint256)" 1000000)

cast send \
  --rpc-url $RPC \
  --private-key $PRIVATE_KEY \
  --create ${BYTECODE}${CONSTRUCTOR_ARGS}

# Example: constructor that takes an owner address and a name
CONSTRUCTOR_ARGS=$(cast abi-encode "constructor(address,string)" 0xYourAddress "MyToken")

cast send \
  --rpc-url $RPC \
  --private-key $PRIVATE_KEY \
  --create ${BYTECODE}${CONSTRUCTOR_ARGS}
```

### Read Contract State (call)

Use `cast call` for read-only queries (no gas cost). Use `--from` to set the caller address (needed if the contract reads `caller()`):

```bash
CONTRACT=0x<deployed-address>
RPC=http://127.0.0.1:8545
FROM=0xYourAddress

# totalSupply() → uint256
cast call $CONTRACT "totalSupply()(uint256)" --rpc-url $RPC --from $FROM

# balanceOf(address) → uint256
cast call $CONTRACT "balanceOf(address)(uint256)" 0xYourAddress --rpc-url $RPC --from $FROM
```

### Write to Contract (send)

Use `cast send` for state-changing transactions:

```bash
# transfer(address,uint256)
cast send $CONTRACT "transfer(address,uint256)" 0xRecipient 1000 \
  --rpc-url $RPC \
  --private-key $PRIVATE_KEY

# mint(address,uint256)
cast send $CONTRACT "mint(address,uint256)" 0xRecipient 1000000 \
  --rpc-url $RPC \
  --private-key $PRIVATE_KEY
```

### Check Events

```bash
# Get Transfer events from recent blocks
cast logs --from-block latest --address $CONTRACT \
  "Transfer(address,address,uint256)" \
  --rpc-url $RPC
```

### Deploying on Polkadot Testnet

Paseo is the Polkadot testnet with `pallet-revive` support for smart contracts.

#### 1. Generate an Ethereum-compatible account

Use any Ethereum wallet (e.g. MetaMask) to generate an account, or use `cast`:

```bash
cast wallet new
```

Save the private key and address.

#### 2. Get testnet tokens

Request free PAS tokens from the [Polkadot Faucet](https://faucet.polkadot.io/). Enter your address, complete the captcha, and submit.

#### 3. Deploy

```bash
RPC=https://eth-rpc-testnet.polkadot.io/
BYTECODE=0x$(xxd -p target/release/my_contract.polkavm | tr -d '\n')

cast send \
  --rpc-url $RPC \
  --private-key $PRIVATE_KEY \
  --gas-limit 5000000 \
  --create $BYTECODE
```

The transaction receipt contains the deployed contract address.

#### 4. Interact

```bash
CONTRACT=0x<deployed-address>
RPC=https://eth-rpc-testnet.polkadot.io/

# Read
cast call $CONTRACT "totalSupply()(uint256)" --rpc-url $RPC --from 0xYourAddress

# Write
cast send $CONTRACT "transfer(address,uint256)" 0xRecipient 1000 \
  --rpc-url $RPC \
  --private-key $PRIVATE_KEY

# Check events
cast logs --from-block latest --address $CONTRACT \
  "Transfer(address,address,uint256)" \
  --rpc-url $RPC
```
