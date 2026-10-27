import { describe, expect, it } from 'vitest'
import type { Abi, AbiParameter } from 'viem'
import { encodeFunctionData } from 'viem/utils'

import { loadAbi } from '../../src/fixtures.js'

// ABI emitter defects this suite found. Like the rest of `known-gaps/`, these
// are real failures, not skips: each states what a viem user needs from the
// ABI, and fixing the emitter turns it green. When one passes, move it into
// the blocking suite (`abi-completeness.test.ts`) and, for the reference
// types, drop `sol-reference-types` from `KNOWN_INVALID_TYPES` in
// `abi-shape.test.ts` — the guard there fails until you do.

describe('a contract emits its fallback handler', () => {
  // solc emits `{"type":"fallback","stateMutability":…}`; the SDK's ABI model
  // has no `fallback` variant, so a contract that accepts arbitrary calldata
  // looks — to any consumer reading the ABI — like it does not. `abi-surface`
  // declares a non-payable `#[fallback]`.
  it('abi-surface declares a non-payable fallback', () => {
    const entry = loadAbi('abi/abi-surface.abi.json').find((item) => item.type === 'fallback')
    expect(entry).toEqual({ type: 'fallback', stateMutability: 'nonpayable' })
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
    // here except a bare type name; rejecting the interface outright (as solc
    // does in an external signature) would also close this gap.
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

function inputsOf(abi: Abi, name: string): readonly AbiParameter[] {
  const item = abi.find((candidate) => candidate.type === 'function' && candidate.name === name)
  expect(item, `${name} is missing from the ABI`).toBeDefined()
  return (item as { inputs: readonly AbiParameter[] }).inputs
}

function inputTypesOf(abi: Abi, name: string): string[] {
  return inputsOf(abi, name).map((param) => param.type)
}
