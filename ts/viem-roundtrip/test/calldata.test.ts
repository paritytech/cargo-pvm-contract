import { describe, expect, it } from 'vitest'
import { decodeFunctionData, encodeFunctionData } from 'viem/utils'

import { toViemValues } from '../src/convert.js'
import { fixtures, loadAbi } from '../src/fixtures.js'
import { resolveFunction } from './helpers.js'

describe('calldata encoding matches viem', () => {
  for (const contract of fixtures.contracts) {
    const abi = loadAbi(contract.abiFile)

    describe(contract.name, () => {
      // The full ABI is passed deliberately, overloads included: viem resolves
      // the item from the argument shape, so this also checks that our two
      // same-named methods stay distinguishable.
      it.each(contract.functions)('encode $id', (fixture) => {
        const item = resolveFunction(abi, fixture)
        const args = toViemValues(item.inputs, fixture.args)
        expect(encodeFunctionData({ abi, functionName: fixture.functionName, args })).toBe(
          fixture.calldata,
        )
      })

      it.each(contract.functions)('decode $id', (fixture) => {
        const item = resolveFunction(abi, fixture)
        const args = toViemValues(item.inputs, fixture.args)
        const decoded = decodeFunctionData({ abi, data: fixture.calldata })
        expect(decoded.functionName).toBe(fixture.functionName)
        // viem omits `args` entirely for a zero-argument function.
        expect(decoded.args ?? []).toEqual(args)
      })
    })
  }
})
