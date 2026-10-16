import { describe, expect, it } from 'vitest'
import { decodeAbiParameters, encodeAbiParameters } from 'viem/utils'

import { toViemValues } from '../src/convert.js'
import { fixtures } from '../src/fixtures.js'

// Raw parameter encoding, checked against viem in both directions. These cases
// carry no selector and belong to no contract.
//
// Scope is deliberately narrow. `pvm-contract-types::tests` already pins every
// primitive and the common container shapes byte-for-byte against alloy-core,
// much of it under proptest, so this suite covers the delta rather than
// restating it: composites and boundaries that differential does not reach, plus
// one value per `SOL_NAME` family. That last part is the reason these cases
// exist at all — their `types` come from `SolEncode::abi_param()`, and nothing
// else in the repo hands that descriptor to a real ABI consumer. (The contract
// suites read their types from the emitted ABI files instead.)
describe('parameter encoding matches viem', () => {
  it.each(fixtures.parameters)('$id', ({ types, values, encoded }) => {
    const viemValues = toViemValues(types, values)
    expect(encodeAbiParameters(types, viemValues)).toBe(encoded)
  })
})

describe('parameter decoding matches viem', () => {
  it.each(fixtures.parameters)('$id', ({ types, values, encoded }) => {
    const viemValues = toViemValues(types, values)
    expect(decodeAbiParameters(types, encoded)).toEqual(viemValues)
  })
})

describe('bytes and uint8[] are not interchangeable', () => {
  // Same Rust shape (`Bytes` vs `Vec<u8>`), same JSON bytes, deliberately
  // different Solidity types. A regression that collapsed the two would make
  // both encode identically, so assert they do not.
  it('encodes the same three bytes differently', () => {
    const packed = fixtures.parameters.find((c) => c.id === 'bytes/1')
    const spread = fixtures.parameters.find((c) => c.id === 'uint8-array/one')
    expect(packed).toBeDefined()
    expect(spread).toBeDefined()
    expect(packed!.types[0]!.type).toBe('bytes')
    expect(spread!.types[0]!.type).toBe('uint8[]')
    expect(packed!.encoded).not.toBe(spread!.encoded)
  })
})
