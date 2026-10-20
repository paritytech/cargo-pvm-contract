import { expect } from 'vitest'
import type { Abi, AbiFunction } from 'viem'
import {
  decodeErrorResult,
  encodeErrorResult,
  formatAbiItem,
  toFunctionSelector,
} from 'viem/utils'

import { toViemValues } from '../src/convert.js'
import type { ErrorCase, FunctionCase } from '../src/fixtures.js'

type AbiError = Extract<Abi[number], { type: 'error' }>

/**
 * Resolve the ABI item a fixture case refers to, by canonical signature.
 *
 * Signature rather than name, because the corpus deliberately contains an
 * overloaded pair and viem's own `getAbiItem` disambiguates by argument shape,
 * which is not a lookup key the fixture has.
 */
export function resolveFunction(abi: Abi, fixture: FunctionCase): AbiFunction {
  const matches = abi.filter(
    (item): item is AbiFunction =>
      item.type === 'function' &&
      item.name === fixture.functionName &&
      formatAbiItem(item) === fixture.signature,
  )
  expect(matches.length, `${fixture.signature} is missing from the ABI`).toBe(1)
  return matches[0]!
}

/** Resolve an error item by canonical signature, failing if it is absent. */
export function findError(abi: Abi, signature: string): AbiError {
  const matches = abi.filter(
    (item): item is AbiError => item.type === 'error' && formatAbiItem(item) === signature,
  )
  expect(matches.length, `${signature} is missing from the ABI`).toBe(1)
  return matches[0]!
}

// The three assertions every error vector is held to. They are shared between
// `errors.test.ts` / `abi-shape.test.ts` (over `errors`, blocking) and
// `known-gaps/errors.test.ts` (over `knownGapErrors`, non-blocking), so a
// known-gap case fails in exactly the way a regular case would and promotes
// without rewriting.

/** The ABI carries the item, under the fixture's name, with the right selector. */
export function expectErrorSignatureAgrees(abi: Abi, fixture: ErrorCase) {
  const item = findError(abi, fixture.signature)
  expect(item.name).toBe(fixture.errorName)
  // `formatAbiItem` is what viem's own `decodeErrorResult` hashes; passing the
  // item straight to `toFunctionSelector` would fold the `error` keyword into
  // the hash and give a different answer.
  expect(toFunctionSelector(formatAbiItem(item))).toBe(fixture.selector)
  // For an error enum the enum's own selector is zeroed, so the leading four
  // bytes of the payload must be the held variant's.
  expect(fixture.data.slice(0, 10)).toBe(fixture.selector)
}

/** viem decodes the contract's real revert bytes through the ABI. */
export function expectErrorDecodes(abi: Abi, fixture: ErrorCase) {
  const decoded = decodeErrorResult({ abi, data: fixture.data })
  expect(decoded.errorName).toBe(fixture.errorName)
  const item = decoded.abiItem as AbiError
  expect(formatAbiItem(item)).toBe(fixture.signature)
  const args = toViemValues(item.inputs ?? [], fixture.args)
  expect(decoded.args ?? []).toEqual(args)
}

/** viem re-encodes the same arguments to the same bytes. */
export function expectErrorEncodes(abi: Abi, fixture: ErrorCase) {
  const item = findError(abi, fixture.signature)
  const args = toViemValues(item.inputs ?? [], fixture.args)
  expect(
    encodeErrorResult({
      abi: [item],
      errorName: fixture.errorName,
      ...(args.length > 0 ? { args } : {}),
    } as never),
  ).toBe(fixture.data)
}
