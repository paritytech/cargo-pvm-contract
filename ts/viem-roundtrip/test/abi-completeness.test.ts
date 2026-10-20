import { describe, expect, it } from 'vitest'
import type { Abi, AbiParameter } from 'viem'
import { encodeAbiParameters, encodeDeployData, formatAbiItem } from 'viem/utils'

import { toViemValues } from '../src/convert.js'
import { loadAbi } from '../src/fixtures.js'

// Everything in this file is about ABI *completeness* rather than encoding: can
// a viem user do the things the contract actually supports, given only the
// emitted `.abi.json`?
//
// The claims here hold today. The ones the emitter cannot yet satisfy live in
// `known-gaps/`, which CI runs without blocking.

describe('a contract emits the errors it can revert with', () => {
  // On the Rust path every `#[derive(SolError)]` type reachable from a method's
  // error enum reaches the ABI, including the framework's `Error(string)` /
  // `Panic(uint256)` wrappers and the reentrancy guard's error.
  it('abi-surface declares every error its enum can hold', () => {
    expect(errorSignatures(loadAbi('abi/abi-surface.abi.json'))).toEqual(
      expect.arrayContaining([
        'Unauthorized()',
        'InsufficientBalance(address,uint256,uint256)',
        'DetailedFailure(string,uint32)',
        'Panic(uint256)',
        'Error(string)',
        'ReentrancyGuardReentrantCall()',
      ]),
    )
  })
})

describe('a contract emits its constructor', () => {
  it('abi-surface constructor arguments encode into deploy data', () => {
    const abi = loadAbi('abi/abi-surface.abi.json')
    const constructor = abi.find((item) => item.type === 'constructor')!
    const inputs = constructor.inputs as readonly AbiParameter[]
    const args = toViemValues(inputs, [
      '0xd8da6bf26964af9d7eed9e03e53415d37aa96045',
      '1000000',
    ])
    const deployData = encodeDeployData({ abi, bytecode: '0x00', args } as never)
    expect(deployData).toBe(`0x00${encodeAbiParameters(inputs, args).slice(2)}`)
  })
})

function errorSignatures(abi: Abi): string[] {
  return abi.filter((item) => item.type === 'error').map((item) => formatAbiItem(item))
}
