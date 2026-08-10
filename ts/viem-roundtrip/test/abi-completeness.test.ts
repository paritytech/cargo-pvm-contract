import { describe, expect, it } from 'vitest'
import type { Abi, AbiParameter } from 'viem'
import {
  encodeAbiParameters,
  encodeDeployData,
  encodeFunctionData,
  formatAbiItem,
} from 'viem/utils'

import { toViemValues } from '../src/convert.js'
import { loadAbi } from '../src/fixtures.js'

// Everything in this file is about ABI *completeness* rather than encoding: can
// a viem user do the things the contract actually supports, given only the
// emitted `.abi.json`?
//
// The failures here are real gaps, not fixture drift. They are asserted rather
// than skipped so that fixing the emitter turns them green, and so that nobody
// has to rediscover them.

describe('a contract emits the errors it can revert with', () => {
  // The `.sol` path derives the whole ABI from the Solidity interface, so a
  // `#[derive(SolError)]` type that the `.sol` does not declare never reaches
  // the ABI — and viem then cannot decode that revert. `ErrorHandling.sol`
  // declares no errors, while `error-handling.rs` reverts with two.
  it('error-handling declares AlwaysReverts and ZeroNotAllowed', () => {
    expect(errorSignatures(loadAbi('abi/error-handling.abi.json'))).toEqual(
      expect.arrayContaining(['AlwaysReverts()', 'ZeroNotAllowed()']),
    )
  })

  it('error-caller declares Error(string)', () => {
    expect(errorSignatures(loadAbi('abi/error-caller.abi.json'))).toContain('Error(string)')
  })

  // Positive control: on the Rust path the same declarations do reach the ABI,
  // which localises the gap above to `.sol` parsing rather than to the emitter.
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
  // Without a constructor entry, `encodeDeployData` cannot append constructor
  // arguments and the contract is undeployable through viem. A Solidity
  // *interface* cannot declare a constructor at all, so the `.sol` path has no
  // way to recover this — the arguments are only known on the Rust side.
  it('constructor-args declares its two constructor inputs', () => {
    const constructor = loadAbi('abi/constructor-args.abi.json').find(
      (item) => item.type === 'constructor',
    )
    expect(constructor).toBeDefined()
    expect(constructor!.inputs).toHaveLength(2)
  })

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

describe('reference types resolve to an ABI type', () => {
  // `type_to_abi_param` falls back to the bare Solidity type name when it cannot
  // resolve a custom type. That produces `type` strings outside the ABI grammar,
  // which viem accepts at import and then throws on at the first encode, decode
  // or selector computation — so the ABI looks fine until it is used.
  const abi = loadAbi('abi/sol-reference-types.abi.json')

  it('a contract-typed parameter becomes address', () => {
    // solc: {"type":"address","internalType":"contract IToken"}. Passing a
    // contract handle is ordinary Solidity, so this shape is common.
    expect(inputTypesOf(abi, 'setToken')).toEqual(['address'])
    expect(inputTypesOf(abi, 'setTokens')).toEqual(['address[]'])
  })

  it('a self-referential struct field resolves', () => {
    // `struct Node { uint256 value; Node[] children; }` — the cycle is broken by
    // emitting `Node[]`, which is not an ABI type. Any resolution is acceptable
    // here except a bare type name; rejecting the interface outright would also
    // be an improvement over emitting an unusable ABI.
    const [param] = inputsOf(abi, 'addNode')
    const components = (param as { components?: readonly AbiParameter[] }).components ?? []
    expect(components[1]?.type).not.toBe('Node[]')
  })

  it('viem can encode a call to every function', () => {
    // The user-facing consequence: an unresolvable `type` survives ABI import
    // and only throws here, at the first attempt to actually call the contract.
    expect(() =>
      encodeFunctionData({
        abi,
        functionName: 'setToken',
        args: ['0xd8da6bf26964af9d7eed9e03e53415d37aa96045'],
      } as never),
    ).not.toThrow()
  })
})

describe('a contract emits its fallback handler', () => {
  // solc emits `{"type":"fallback","stateMutability":…}`; the SDK's ABI model
  // has no `fallback` variant, so a contract that accepts arbitrary calldata
  // looks — to any consumer reading the ABI — like it does not.
  it('abi-surface declares a fallback', () => {
    const abi = loadAbi('abi/abi-surface.abi.json')
    expect(abi.some((item) => item.type === 'fallback')).toBe(true)
  })
})

function inputsOf(abi: Abi, name: string): readonly AbiParameter[] {
  const item = abi.find((candidate) => candidate.type === 'function' && candidate.name === name)
  expect(item, `${name} is missing from the ABI`).toBeDefined()
  return (item as { inputs: readonly AbiParameter[] }).inputs
}

function inputTypesOf(abi: Abi, name: string): string[] {
  return inputsOf(abi, name).map((param) => param.type)
}

function errorSignatures(abi: Abi): string[] {
  return abi.filter((item) => item.type === 'error').map((item) => formatAbiItem(item))
}
