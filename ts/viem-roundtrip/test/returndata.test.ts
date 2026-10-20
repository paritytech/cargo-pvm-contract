import { describe, expect, it } from 'vitest'
import { decodeFunctionResult, encodeFunctionResult } from 'viem/utils'

import { expectedResult, toViemValues } from '../src/convert.js'
import { fixtures, loadAbi } from '../src/fixtures.js'
import { resolveFunction } from './helpers.js'

describe('return-data decoding matches viem', () => {
  for (const contract of fixtures.contracts) {
    const abi = loadAbi(contract.abiFile)
    const withReturns = contract.functions.filter((f) => f.returndata !== undefined)
    if (withReturns.length === 0) continue

    describe(contract.name, () => {
      it.each(withReturns)('decode $id', (fixture) => {
        const item = resolveFunction(abi, fixture)
        const args = toViemValues(item.inputs, fixture.args)
        const decoded = decodeFunctionResult({
          abi,
          functionName: fixture.functionName,
          args,
          data: fixture.returndata!,
        })
        expect(decoded).toEqual(expectedResult(item.outputs, fixture.result))
      })

      it.each(withReturns)('encode $id', (fixture) => {
        const item = resolveFunction(abi, fixture)
        // Narrowed to the resolved item: `encodeFunctionResult` looks the
        // function up by name only, so it cannot tell overloads apart.
        expect(
          encodeFunctionResult({
            abi: [item],
            functionName: fixture.functionName,
            result: expectedResult(item.outputs, fixture.result) as never,
          }),
        ).toBe(fixture.returndata)
      })
    })
  }
})

describe('viem returns the shape the ABI implies', () => {
  const abi = loadAbi('abi/return-values.abi.json')

  it('one output decodes to a bare value', () => {
    const fixture = fixtures.contracts
      .find((c) => c.name === 'return-values')!
      .functions.find((f) => f.id === 'identity')!
    expect(decodeFunctionResult({ abi, functionName: 'identity', data: fixture.returndata! })).toBe(
      7n,
    )
  })

  it('several outputs decode to an array, not a tuple object', () => {
    const fixture = fixtures.contracts
      .find((c) => c.name === 'return-values')!
      .functions.find((f) => f.id === 'getPair')!
    expect(decodeFunctionResult({ abi, functionName: 'getPair', data: fixture.returndata! })).toEqual(
      [42n, true],
    )
  })

  it('a single tuple output decodes to an object', () => {
    const surface = loadAbi('abi/abi-surface.abi.json')
    const fixture = fixtures.contracts
      .find((c) => c.name === 'abi-surface')!
      .functions.find((f) => f.id === 'echoPair')!
    expect(
      decodeFunctionResult({ abi: surface, functionName: 'echoPair', data: fixture.returndata! }),
    ).toEqual({ lo: 7n, hi: 8n })
  })
})
