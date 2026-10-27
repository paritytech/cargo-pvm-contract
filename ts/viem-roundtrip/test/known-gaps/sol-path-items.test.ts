import { describe, expect, it } from 'vitest'
import type { Abi, AbiParameter } from 'viem'
import { encodeDeployData, formatAbiItem } from 'viem/utils'

import { toViemValues } from '../../src/convert.js'
import { fixtures, loadAbi } from '../../src/fixtures.js'

// Assertions the emitter does not satisfy yet. They are real failures, not
// skips: each one states what a viem user needs from the ABI, and fixing the
// emitter turns it green. CI runs this directory as a separate, non-blocking
// step (`pnpm test:known-gaps`) so the gap stays visible without holding every
// unrelated change hostage to it. When the last test here passes, move it back
// into `test/abi-completeness.test.ts` and delete the directory.
//
// The gap: when `#[contract("X.sol")]` names an interface, the whole ABI is
// derived from that file. A `#[derive(SolError)]` type, a `#[fallback]` /
// `#[receive]` handler or a `#[constructor]` signature that the `.sol` does not
// declare never reaches the ABI, so viem cannot decode the revert, see the
// handler, or deploy the contract. A Solidity *interface* can declare errors,
// `fallback` and `receive` — so those could be closed by requiring the `.sol`
// to list what the Rust side defines — but it cannot declare a constructor, so
// that half needs the arguments merged in from the Rust side.
//
// The revert payloads themselves are exercised through viem's decoder in
// `errors.test.ts` next to this file; the tests here cover the item-level
// claims that have no byte vector.

describe('a .sol-backed contract emits the custom errors it can revert with', () => {
  // `error-handling.rs` reverts with two custom errors; `ErrorHandling.sol`
  // declares none. (The Solidity built-ins `Error(string)` / `Panic(uint256)`
  // are not part of this gap: solc never emits them and viem decodes them
  // without an entry — see `errors.test.ts`.)
  it('error-handling declares AlwaysReverts and ZeroNotAllowed', () => {
    expect(errorSignatures(loadAbi('abi/error-handling.abi.json'))).toEqual(
      expect.arrayContaining(['AlwaysReverts()', 'ZeroNotAllowed()']),
    )
  })
})

describe('a .sol-backed contract emits its fallback handler', () => {
  // `error-handling.rs` has a `#[fallback]`; `ErrorHandling.sol` does not
  // declare `fallback() external`. Unlike errors and the constructor, an
  // interface *can* declare this, so the Rust-side fix is the same `.sol`
  // consistency check the macro already runs for functions.
  it('error-handling declares a non-payable fallback', () => {
    const entry = loadAbi('abi/error-handling.abi.json').find((item) => item.type === 'fallback')
    expect(entry).toEqual({ type: 'fallback', stateMutability: 'nonpayable' })
  })
})

describe('a .sol-backed contract can be deployed through viem', () => {
  // The user-facing consequence of the missing constructor item: with the real
  // constructor arguments, `encodeDeployData` must append their encoding to the
  // bytecode. Without the item viem either refuses the args or drops them, and
  // the deployment would run the constructor on empty calldata.
  const withDeploy = fixtures.contracts.filter((c) => c.knownGapDeploy !== undefined)

  it.each(withDeploy)('$name deploys with its constructor arguments', (contract) => {
    const abi = loadAbi(contract.abiFile)
    const deploy = contract.knownGapDeploy!
    const constructor = abi.find((item) => item.type === 'constructor')
    expect(constructor, `${contract.name} has no constructor item`).toBeDefined()
    const inputs = constructor!.inputs as readonly AbiParameter[]
    const args = toViemValues(inputs, deploy.args)
    expect(encodeDeployData({ abi, bytecode: '0x00', args } as never)).toBe(
      `0x00${deploy.encodedArgs.slice(2)}`,
    )
  })
})

function errorSignatures(abi: Abi): string[] {
  return abi.filter((item) => item.type === 'error').map((item) => formatAbiItem(item))
}
