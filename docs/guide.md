# pvm-contract — Quick Start Guide

Write Polkadot smart contracts in Rust, targeting PolkaVM (RISC-V) with Solidity ABI compatibility.

## Setup

1. Install (requires Parity auth):

```
git clone -b charles/cdm-integration https://github.com/paritytech/cargo-pvm-contract.git /tmp/cargo-pvm-contract && cargo install --force --locked --path /tmp/cargo-pvm-contract/crates/cargo-pvm-contract && rm -rf /tmp/cargo-pvm-contract
```

Create a template & build it:

```bash
# Select "alloy-core + allocator" in the selections
cargo pvm-contract init --init-type example --example counter

cd <project-name>
cargo pvm-contract build    # produces .polkavm + .abi.json
```

## Minimal Contract

```rust
#![no_main]
#![no_std]

use pvm_contract as pvm;

#[pvm::storage]
struct Storage {
    count: u32,
}

#[pvm::contract]
mod counter {
    use super::*;

    #[pvm::constructor]
    pub fn new() -> Result<(), Error> {
        Storage::count().set(&0);
        Ok(())
    }

    #[pvm::method]
    pub fn increment() {
        let current = Storage::count().get().unwrap_or(0);
        Storage::count().set(&(current + 1));
    }

    #[pvm::method]
    pub fn get_count() -> u32 {
        Storage::count().get().unwrap_or(0)
    }
}
```

## Storage

Declare with `#[pvm::storage]` outside the contract module. Fields become static accessors — `Lazy<T>` for values, `Mapping<K, V>` for maps.

```rust
#[pvm::storage]
struct Storage {
    owner: Address,                       // Lazy<Address>
    balances: Mapping<Address, u64>,      // Mapping<Address, u64>
}

// Read / write
Storage::owner().set(&pvm::caller());
let bal = Storage::balances().get(&addr).unwrap_or(0);
Storage::balances().set(&addr, &100);
```

Multiple storage structs are allowed:

```rust
#[pvm::storage]
struct JobPostings {
    budget: Mapping<u64, u64>,
}
```

## Custom Structs

```rust
#[derive(pvm::SolAbi)]
struct Point {
    x: u64,
    y: u64,
}
```

Structs with `SolAbi` can be used as method parameters, return types, and storage values.

## Cross-Contract Calls

Every `#[pvm::contract]` auto-generates a `Reference` type. Add the other contract as a Cargo dependency, then call it:

```rust
let rep = reputation::reference(address);
let avg = rep.get_average_rating(subject)?;   // CallResult<u64>
```

References are encodable, so you can store them:

```rust
#[pvm::storage]
struct Contracts {
    reputation: reputation::Reference,
}
```

## Types

| Rust                            | Solidity                             |
| ------------------------------- | ------------------------------------ |
| `Address` (H160)                | `address`                            |
| `U256` / `I256`                 | `uint256` / `int256`                 |
| `u8`..`u128`, `i8`..`i128`      | `uint8`..`uint128`, `int8`..`int128` |
| `bool`                          | `bool`                               |
| `String`                        | `string`                             |
| `FixedBytes<N>`                 | `bytesN`                             |
| `#[derive(pvm::SolAbi)]` struct | `tuple`                              |

## Attributes Reference

| Attribute                | Where    | Purpose                                                      |
| ------------------------ | -------- | ------------------------------------------------------------ |
| `#[pvm::contract]`       | `mod`    | Marks the contract module, generates entry points + dispatch |
| `#[pvm::constructor]`    | `fn`     | Deploy-time initialization, returns `Result<(), Error>`      |
| `#[pvm::method]`         | `fn`     | Callable function (Keccak selector auto-derived from name)   |
| `#[pvm::fallback]`       | `fn`     | Called when no selector matches                              |
| `#[pvm::storage]`        | `struct` | Generates static storage accessors                           |
| `#[derive(pvm::SolAbi)]` | `struct` | ABI encoding/decoding for custom types                       |

## Deployment (PAPI)

Minimal steps to create a typescript deployer to deploy a `.polkavm` contract using [polkadot-api](https://papi.how):

```
bun init .
bun install polkadot-api @polkadot-labs/hdkd-helpers @polkadot-labs/hdkd
npx papi add assetHub -n paseo_asset_hub
```

Create `deploy.ts`:

```typescript
import { createClient, Binary } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws-provider";
import { withPolkadotSdkCompat } from "polkadot-api/polkadot-sdk-compat";
import { assetHub } from "@polkadot-api/descriptors";
import { readFileSync } from "fs";
import { sr25519CreateDerive } from "@polkadot-labs/hdkd";
import {
    DEV_PHRASE,
    entropyToMiniSecret,
    mnemonicToEntropy,
} from "@polkadot-labs/hdkd-helpers";
import { getPolkadotSigner } from "polkadot-api/signer";

// Prepare signer (dev account)
const derive = sr25519CreateDerive(
    entropyToMiniSecret(mnemonicToEntropy(DEV_PHRASE)),
);
const keyPair = derive("//Alice");
const signer = getPolkadotSigner(keyPair.publicKey, "Sr25519", keyPair.sign);

// Create Paseo Assethub client
console.log("Connecting to Paseo Asset Hub...");
const client = createClient(
    withPolkadotSdkCompat(
        getWsProvider("wss://asset-hub-paseo-rpc.n.dwellir.com"),
    ),
);
const api = client.getTypedApi(assetHub);

// Deploy
console.log("Deploying contract...");
const bytecode = readFileSync("target/counter.release.polkavm");
const result = await api.tx.Revive.instantiate_with_code({
    value: 0n,
    weight_limit: { ref_time: 500_000_000_000n, proof_size: 2_000_000n },
    storage_deposit_limit: 10_000_000_000_000n,
    code: Binary.fromBytes(bytecode),
    data: Binary.fromBytes(new Uint8Array(0)),
    salt: undefined,
}).signAndSubmit(signer);

console.log("Awaiting response...");
const deployAddress = api.event.Revive.Instantiated.filter(
    result.events,
)[0]?.contract.asHex();
console.log("Deployed to:", deployAddress);

client.destroy();
```

Finally, `bun deploy.ts`
