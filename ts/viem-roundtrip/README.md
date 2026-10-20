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
| `test/abi-shape.test.ts` | Structural conformance, the signature gate, the grammar gate, fixture-vs-ABI agreement, and the vector-coverage gate |
| `test/abi-completeness.test.ts` | Whether the ABI describes everything the contract can do: errors and constructor on the Rust path |
| `test/known-gaps/` | viem decode / deploy assertions the emitter does not satisfy yet — run separately, see below |
| `test/types.test-d.ts` | abitype inference, enforced by `tsc` |

## Gates

Three gates keep coverage from eroding, in increasing cost:

- **Every ABI item has a canonical signature viem can hash.** `abi-shape.test.ts`
  runs `formatAbiItem` and the selector helpers over every function, error and
  event of every emitted ABI. This needs no hand-written values, so it covers
  all 22 ABIs, and it is the one check that ties the JSON artifact (with its
  tuple `components`) back to the selectors `const_selector` produced.
- **Every parameter type is a valid ABI type.** viem parses an ABI lazily, so an
  unrepresentable `type` string survives import and only throws at the first
  encode. The gate walks every parameter of every item, nested tuples included.
- **Every item of a contract with vectors is exercised.** Vector coverage is
  opt-in by contract: an ABI with a corpus entry in `vectors.json` must have a
  case for every function, error and event it declares, with the handful of
  genuine exceptions listed in `UNEXERCISED_BY_DESIGN` with a reason. An ABI
  without a corpus entry is covered by the two gates above only — it forgoes the
  byte-level round-trip, which is only worth writing for a contract that adds a
  parameter shape.

`SolEncode`/`SolDecode` symmetry is checked on the Rust side instead, inside
`parameter_case`: viem cannot see an encoder whose output our own dispatch then
misreads, so the generator decodes each value it encodes and compares.

### What this suite deliberately does not re-test

Two Rust test files already pin encoding bytes against alloy-core, much of it
under proptest: `pvm-contract-types/src/tests.rs` (every primitive and the
common container shapes) and `pvm-contract-macros/tests/derive_sol_type_encoding.rs`
(`#[derive(SolType)]` structs, static and dynamic, nested, in arrays and in
tuples). `sol_error_derive.rs` and `sol_event_derive.rs` do the same for revert
payloads and log topics. `parameters.test.ts` does not restate any of it — a
second set of hand-written vectors would be a weaker version of the same claim
with more to keep in sync. It covers the delta instead: a handful of composite
shapes none of those files reach, and the descriptor path
(`SolEncode::abi_param`) that the alloy tests bypass by hand-writing types.

The same reasoning sets the size of the contract corpus. Of the 22 emitted ABIs,
14 carry vectors; the other eight (`storage-types`, `caller-check`, `receive`,
`flipper-call`, `flipper-delegate`, `point-adder-call`, `precompiles`,
`sol-reference-types`) add no parameter shape the 14 do not already carry, so
they are covered by the signature and grammar gates only. `constructor-args` is
among the 14 only for its known-gap deploy case.

The `.sol` fixtures under `crates/pvm-viem-roundtrip/sol/` are standalone
interfaces with no Rust implementation — the builder derives an ABI by parsing
them, which is enough to drive `type_to_abi_param` over every branch of the
Solidity type mapping (enums, user-defined value types, nested structs, struct
arrays, bare `uint`/`int`, nested array suffixes, contract-typed parameters).

The runtime suites read `fixtures/` off disk rather than importing
`src/generated/abis.ts`, so a bug in the codegen script cannot make a runtime
test pass against something the Rust side never emitted. The generated module
exists for the type tests, where a `const` literal is the whole point — a JSON
import widens to `string` and `unknown[]`, and every type assertion would then
pass vacuously.

## Running

```bash
pnpm install --frozen-lockfile
pnpm test              # runtime round-trips (blocking in CI)
pnpm typecheck         # abitype inference over the emitted ABIs (blocking in CI)
pnpm test:known-gaps   # assertions the emitter does not satisfy yet (non-blocking in CI)
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

## Known gaps

`test/known-gaps/` holds assertions that are red today. They state what a viem
user needs from the ABI rather than what the emitter produces, so fixing the
emitter turns them green. They are real failures, not skips; CI runs them as a
separate `continue-on-error` step so the gap stays visible without blocking
unrelated changes.

The evidence is viem's, not a shape check. `known-gaps/errors.test.ts` runs the
same decode, encode and signature assertions as the blocking suite over
`knownGapErrors` — revert payloads the contract really produces, encoded by its
own `SolError` impl — so each failure is viem's `AbiErrorSignatureNotFoundError`
on the contract's actual bytes. `known-gaps/sol-path-items.test.ts` drives
`encodeDeployData` with the real constructor arguments from `knownGapDeploy`.

The arrangement cannot go stale: a guard in `abi-shape.test.ts` fails the
blocking suite the moment an ABI gains an item a known-gap vector refers to,
with a message saying to move the vector from `knownGapErrors` to `errors` in
`corpus.rs`. The assertions are shared, so that move is the whole promotion.
When the last test in the directory passes, delete it.

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
  `encodeFunctionData`. `sol-reference-types` is therefore exempt from the
  blocking grammar gate, and a guard fails the blocking suite once its types
  become valid, so the exemption cannot outlive the fix.
- **`.sol`-path ABIs omit Rust-declared items.** When `#[contract("X.sol")]`
  names an interface, the whole ABI is derived from that file, so a
  `#[derive(SolError)]` type, a `#[fallback]` / `#[receive]` handler or a
  `#[constructor]` signature the `.sol` does not declare never reaches the ABI.
  `error-handling` reverts with `AlwaysReverts()` / `ZeroNotAllowed()` that
  viem cannot decode and has a `#[fallback]` the ABI does not show, and
  `constructor-args` cannot be deployed through `encodeDeployData` because its
  two constructor arguments are absent. A Solidity *interface* can declare
  errors, `fallback` and `receive`, so those could be closed by requiring the
  `.sol` to list what the Rust side defines (the same consistency check the
  macro already runs for functions); it cannot declare a constructor, so that
  half needs the arguments merged in from the Rust side.

  The Solidity built-ins `Error(string)` and `Panic(uint256)` are *not* part
  of this gap: solc never emits them in an ABI, and viem decodes both without
  an entry. `errors.test.ts` asserts that against the `error-caller` ABI, whose
  interface declares neither.

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
