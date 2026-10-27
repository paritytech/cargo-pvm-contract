import { describe, expectTypeOf, it } from 'vitest'
import { decodeFunctionResult, encodeFunctionData } from 'viem/utils'

import {
  abiSurfaceAbi,
  flipperAbi,
  multiMethodAbi,
  pointAdderAbi,
  returnValuesAbi,
} from '../src/generated/abis.js'

// The runtime suites prove viem *behaves* correctly on our ABI. This one proves
// the ABI is useful to a TypeScript author: that abitype can infer function
// names, argument types and return types from it.
//
// These assertions only mean something because `abis.ts` holds `as const`
// literals. A JSON import — even with `resolveJsonModule` — widens to `string`
// and `unknown[]`, at which point `functionName` accepts anything and every
// assertion below would pass vacuously.

describe('function names are inferred', () => {
  it('accepts a declared name and rejects anything else', () => {
    encodeFunctionData({ abi: flipperAbi, functionName: 'flip' })
    // @ts-expect-error — 'flipp' is not a function on this ABI.
    encodeFunctionData({ abi: flipperAbi, functionName: 'flipp' })
  })
})

describe('argument types are inferred', () => {
  it('uint256 arguments are bigint', () => {
    encodeFunctionData({ abi: multiMethodAbi, functionName: 'add', args: [1n, 2n] })
    // @ts-expect-error — uint256 is bigint, not string.
    encodeFunctionData({ abi: multiMethodAbi, functionName: 'add', args: ['1', '2'] })
    // @ts-expect-error — `add` takes two arguments.
    encodeFunctionData({ abi: multiMethodAbi, functionName: 'add', args: [1n] })
  })

  it('narrow integers are number and wide ones are bigint', () => {
    // abitype maps intN/uintN up to 48 bits to `number` and wider ones to
    // `bigint`; `echoInts` spans the boundary in a single signature.
    encodeFunctionData({
      abi: abiSurfaceAbi,
      functionName: 'echoInts',
      args: [-1, -1, -1, -1n, -1n, -1n],
    })
    encodeFunctionData({
      abi: abiSurfaceAbi,
      functionName: 'echoInts',
      // @ts-expect-error — int8 is `number` at this width, not bigint.
      args: [-1n, -1, -1, -1n, -1n, -1n],
    })
  })

  it('a struct argument is an object keyed by component name', () => {
    encodeFunctionData({
      abi: pointAdderAbi,
      functionName: 'add',
      args: [
        { a: 1n, b: 2n },
        { a: 3n, b: 4n },
      ],
    })
    // @ts-expect-error — the components are named `a` and `b`.
    encodeFunctionData({ abi: pointAdderAbi, functionName: 'add', args: [{ x: 1n, y: 2n }] })
  })
})

describe('return types are inferred', () => {
  const data = '0x' as const

  it('a single output infers the bare value', () => {
    expectTypeOf(
      decodeFunctionResult({ abi: flipperAbi, functionName: 'get', data }),
    ).toEqualTypeOf<boolean>()
    expectTypeOf(
      decodeFunctionResult({ abi: multiMethodAbi, functionName: 'getCounter', data }),
    ).toEqualTypeOf<bigint>()
    expectTypeOf(
      decodeFunctionResult({ abi: abiSurfaceAbi, functionName: 'version', data }),
    ).toEqualTypeOf<number>()
  })

  it('several outputs infer a tuple', () => {
    expectTypeOf(
      decodeFunctionResult({ abi: returnValuesAbi, functionName: 'getPair', data }),
    ).toEqualTypeOf<readonly [bigint, boolean]>()
    expectTypeOf(
      decodeFunctionResult({ abi: returnValuesAbi, functionName: 'getTriple', data }),
    ).toEqualTypeOf<readonly [bigint, `0x${string}`, boolean]>()
  })

  it('a struct output infers an object', () => {
    expectTypeOf(
      decodeFunctionResult({ abi: pointAdderAbi, functionName: 'add', data }),
    ).toEqualTypeOf<{ a: bigint; b: bigint }>()
  })

  it('dynamic outputs infer their JavaScript shapes', () => {
    expectTypeOf(
      decodeFunctionResult({ abi: abiSurfaceAbi, functionName: 'echoStrings', data }),
    ).toEqualTypeOf<readonly string[]>()
    expectTypeOf(
      decodeFunctionResult({ abi: abiSurfaceAbi, functionName: 'echoFixedUints', data }),
    ).toEqualTypeOf<readonly [bigint, bigint, bigint]>()
  })
})
