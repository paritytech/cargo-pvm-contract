import { expect } from 'vitest'
import type { Abi, AbiFunction } from 'viem'
import { formatAbiItem } from 'viem/utils'

import type { FunctionCase } from '../src/fixtures.js'

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
