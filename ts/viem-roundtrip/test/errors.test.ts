import { describe, expect, it } from 'vitest'
import type { Abi } from 'viem'
import { decodeErrorResult, encodeErrorResult, formatAbiItem } from 'viem/utils'

import { toViemValues } from '../src/convert.js'
import { fixtures, loadAbi } from '../src/fixtures.js'

type AbiError = Extract<Abi[number], { type: 'error' }>

describe('revert payloads decode with viem', () => {
  for (const contract of fixtures.contracts) {
    if (contract.errors.length === 0) continue
    const abi = loadAbi(contract.abiFile)

    describe(contract.name, () => {
      it.each(contract.errors)('decode $id', (fixture) => {
        const decoded = decodeErrorResult({ abi, data: fixture.data })
        expect(decoded.errorName).toBe(fixture.errorName)
        const item = decoded.abiItem as AbiError
        expect(formatAbiItem(item)).toBe(fixture.signature)
        const args = toViemValues(item.inputs ?? [], fixture.args)
        expect(decoded.args ?? []).toEqual(args)
      })

      it.each(contract.errors)('encode $id', (fixture) => {
        const item = findError(abi, fixture.signature)
        const args = toViemValues(item.inputs ?? [], fixture.args)
        expect(
          encodeErrorResult({
            abi: [item],
            errorName: fixture.errorName,
            ...(args.length > 0 ? { args } : {}),
          } as never),
        ).toBe(fixture.data)
      })
    })
  }
})

describe('an error enum reverts with the held variant, not the enum', () => {
  // `SolError` zeroes an enum's own selector, so a payload encoded through the
  // enum has to carry the inner error's selector. If the enum ever leaked its
  // own (all-zero) selector, viem would fail to find a matching ABI item.
  const abi = loadAbi('abi/abi-surface.abi.json')
  const surface = fixtures.contracts.find((c) => c.name === 'abi-surface')!

  it.each(surface.errors)('$id resolves to a distinct variant', (fixture) => {
    expect(fixture.data.slice(0, 10)).not.toBe('0x00000000')
    expect(decodeErrorResult({ abi, data: fixture.data }).errorName).toBe(fixture.errorName)
  })
})

describe('the standard Solidity reverts keep their canonical selectors', () => {
  const abi = loadAbi('abi/abi-surface.abi.json')

  it('Error(string) is 0x08c379a0', () => {
    const fixture = byId('abi-surface', 'RevertString')
    expect(fixture.data.slice(0, 10)).toBe('0x08c379a0')
    expect(decodeErrorResult({ abi, data: fixture.data })).toMatchObject({
      errorName: 'Error',
      args: ['guard tripped'],
    })
  })

  it('Panic(uint256) is 0x4e487b71 and carries the panic code', () => {
    const overflow = byId('abi-surface', 'Panic/overflow')
    const divByZero = byId('abi-surface', 'Panic/division-by-zero')
    expect(overflow.data.slice(0, 10)).toBe('0x4e487b71')
    expect(decodeErrorResult({ abi, data: overflow.data }).args).toEqual([0x11n])
    expect(decodeErrorResult({ abi, data: divByZero.data }).args).toEqual([0x12n])
  })

  it('ReentrancyGuardReentrantCall matches OpenZeppelin v5', () => {
    // OZ v5's selector, so Foundry and Etherscan name the revert rather than
    // showing raw bytes.
    expect(byId('abi-surface', 'ReentrancyGuardReentrantCall').data).toBe('0x3ee5aeb5')
  })
})

describe('dispatch-level reverts are decodable too', () => {
  // The framework errors are appended to every emitted ABI, so a caller that
  // hits an unknown selector gets a named error rather than opaque bytes.
  it('UnknownSelector()', () => {
    const abi = loadAbi('abi/abi-surface.abi.json')
    const fixture = byId('abi-surface', 'UnknownSelector')
    expect(decodeErrorResult({ abi, data: fixture.data })).toMatchObject({
      errorName: 'UnknownSelector',
    })
  })
})

function byId(contractName: string, id: string) {
  const contract = fixtures.contracts.find((c) => c.name === contractName)!
  const fixture = contract.errors.find((e) => e.id === id)
  if (!fixture) throw new Error(`no error fixture ${contractName}/${id}`)
  return fixture
}

function findError(abi: Abi, signature: string): AbiError {
  const matches = abi.filter(
    (item): item is AbiError => item.type === 'error' && formatAbiItem(item) === signature,
  )
  expect(matches.length, `${signature} is missing from the ABI`).toBe(1)
  return matches[0]!
}
