# viem round-trip suite

Checks the ABI JSON this SDK emits against [viem](https://viem.sh), the ABI
library most TypeScript dApps use. Three claims are under test:

1. **viem can parse our ABI.** Every emitted `.abi.json` is a valid `Abi`, with
   `components` on every tuple, `stateMutability` on every callable, and
   `inputs`/`outputs` present even when empty.
2. **viem's bytes are our bytes.** For a fixed corpus of values, viem's
   `encodeAbiParameters` / `encodeFunctionData` / `encodeErrorResult` /
   `encodeEventTopics` produce exactly what the SDK's `SolEncode`, `SolError`
   and `SolEvent` produce — and the matching decoders round-trip back.
3. **The ABI is useful to a TypeScript author.** abitype infers function names,
   argument types and return types from it.

Everything runs offline. There is no node, no riscv build and no PolkaVM link:
`.sol`-backed ABIs are derived by parsing the Solidity interface, and the golden
vectors come from a host-side Rust binary.

## Layout

| Path | Purpose |
|---|---|
| `fixtures/abi/*.abi.json` | Emitted ABIs, byte-identical to what a real build writes to `target/{profile}/{bin}.abi.json` |
| `fixtures/vectors.json` | Golden encodings produced by the SDK's own traits |
| `src/generated/abis.ts` | The same ABIs as `as const satisfies Abi` literals, for the type tests |
| `src/convert.ts` | Fixture JSON → viem input values (integers become `number` below 56 bits, `bigint` above) |
| `test/parameters.test.ts` | `abi_param` descriptor output, plus the composite shapes the alloy differential does not reach |
| `test/calldata.test.ts` | Selector + argument encoding, overloads included |
| `test/returndata.test.ts` | Single, multi and tuple return shapes |
| `test/errors.test.ts` | Custom errors, error-enum dispatch, `Error(string)`, `Panic(uint256)`, the OZ guard error, framework errors |
| `test/events.test.ts` | Topic hashing, indexed dynamic and composite fields, anonymous events |
| `test/abi-shape.test.ts` | Structural conformance, ABI type-name validity, fixture-vs-ABI signature agreement, and the coverage gate |
| `test/abi-completeness.test.ts` | Whether the ABI describes everything the contract can actually do |
| `test/types.test-d.ts` | abitype inference, enforced by `tsc` |

Two gates keep coverage from eroding:

- **Every ABI item is exercised.** `abi-shape.test.ts` compares the set of
  functions, errors and events in each emitted ABI against the fixture cases and
  fails on anything unaccounted for. The handful of genuine exceptions are listed
  in `UNEXERCISED_BY_DESIGN` with a reason, so adding a contract method without a
  vector is a failure rather than a silent gap.
- **Every parameter type is a valid ABI type.** viem parses an ABI lazily, so an
  unrepresentable `type` string survives import and only throws at the first
  encode or decode. The gate walks every parameter of every item, nested tuples
  included.

`SolEncode`/`SolDecode` symmetry is checked on the Rust side instead, inside
`parameter_case`: viem cannot see an encoder whose output our own dispatch then
misreads, so the generator decodes each value it encodes and compares.

### What this suite deliberately does not re-test

`pvm-contract-types::tests` is a 126-function byte-exact differential against
alloy-core, much of it under proptest, covering every primitive and the common
container shapes. `parameters.test.ts` does not restate it — a second set of
hand-written vectors would be a weaker version of the same claim with more to
keep in sync. It covers the delta instead, and the descriptor path
(`SolEncode::abi_param`) that the alloy tests bypass by hand-writing types.

The same reasoning applies elsewhere: `RevertString` / `Panic` wire format and
event topic packing are pinned against Solidity in that file, so the suites here
test them *through the emitted ABI* — that a viem user can decode the revert or
the log — rather than re-checking the bytes.

The `.sol` fixtures under `crates/pvm-viem-roundtrip/sol/` are standalone
interfaces with no Rust implementation — the builder derives an ABI by parsing
them, which is enough to drive `type_to_abi_param` over every branch of the
Solidity type mapping (enums, user-defined value types, nested structs, struct
arrays, bare `uint`/`int`, nested array suffixes).

The runtime suites read `fixtures/` off disk rather than importing
`src/generated/abis.ts`, so a bug in the codegen script cannot make a runtime
test pass against something the Rust side never emitted. The generated module
exists for the type tests, where a `const` literal is the whole point — a JSON
import widens to `string` and `unknown[]`, and every type assertion would then
pass vacuously.

## Running

```bash
pnpm install --frozen-lockfile
pnpm test        # runtime round-trips
pnpm typecheck   # abitype inference over the emitted ABIs
```

## Regenerating the fixtures

The fixtures are checked in, and CI fails on any diff, so an ABI or encoding
change lands as a reviewable diff rather than being silently absorbed. To
re-bless a legitimate change:

```bash
cargo run -p pvm-viem-roundtrip --bin gen-viem-fixtures
pnpm --dir ts/viem-roundtrip gen:abis
```

then commit the result.

## Known failures

Some tests are red, on purpose. They assert what a viem user needs rather than
what the emitter currently produces, so fixing the emitter turns them green.

- **`.sol`-path ABIs omit Rust-declared items.** When `#[contract("X.sol")]`
  names an interface, the whole ABI is derived from that file, so a
  `#[derive(SolError)]` type or a `#[constructor]` signature the `.sol` does not
  declare never reaches the ABI. `error-handling` reverts with
  `AlwaysReverts()` / `ZeroNotAllowed()` that viem cannot decode, and
  `constructor-args` cannot be deployed through `encodeDeployData` because its
  two constructor arguments are absent. A Solidity *interface* cannot declare a
  constructor at all, so the fix has to come from merging the Rust-derived items
  rather than from editing the `.sol` files.
- **No `fallback` entry.** The ABI model has no `fallback` variant, so a
  contract with a `#[fallback]` handler looks, to any ABI consumer, like it
  rejects unknown calldata. solc emits
  `{"type":"fallback","stateMutability":…}`.
- **Reference types fall back to their bare Solidity name.** When
  `type_to_abi_param` cannot resolve a custom type it emits the type's name as
  the ABI `type`, which is outside the ABI grammar. Two shapes reach it:
  - A **contract- or interface-typed parameter** — `function setToken(IToken t)`
    emits `"type": "IToken"`. solc emits `address` and records the original in
    `internalType`. Passing a contract handle is ordinary Solidity, so this is
    the more consequential of the two.
  - A **self-referential struct** — `struct Node { uint256 v; Node[] kids; }`
    emits `"type": "Node[]"` for the field.

  Both ABIs import into viem without complaint and then throw at the first
  `encodeFunctionData`. Rejecting such an interface at build time would also
  resolve the tests; emitting an ABI that cannot be used is the part being
  asserted against.

### Limits recorded rather than asserted

Two Solidity shapes have no Rust counterpart, so no encoding vector can exist for
them. They stay in `sol/SolTypeSurface.sol` because the parser must still map
them, and they are listed in `UNEXERCISED_BY_DESIGN`:

- Non-canonical integer widths (`uint24`, `uint40`, `int72`, `uint200`): the SDK
  maps only 8/16/32/64/128/256.
- A *fixed* array of `uint8`: `[u8; N]` is `bytesN` and `Vec<u8>` is `uint8[]`,
  so `uint8[N]` is unreachable.

## Boundaries that are viem's, not ours

Asserted so the limits stay documented rather than rediscovered:

- `encodeEventTopics` throws on an **indexed tuple or array** — viem cannot
  build a log filter for a hashed composite. Such an event still decodes.
- `decodeEventLog` cannot read an **anonymous event**: it finds the ABI item by
  `topics[0]`, which an anonymous event does not have.
- An **indexed `string` or `bytes`** is stored as `keccak256(value)`, so the
  value is not recoverable from the log; viem returns the hash.
