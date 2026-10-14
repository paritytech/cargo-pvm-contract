import { describe, it } from 'vitest'

import { fixtures, loadAbi } from '../../src/fixtures.js'
import {
  expectErrorDecodes,
  expectErrorEncodes,
  expectErrorSignatureAgrees,
} from '../helpers.js'

// The same three assertions `errors.test.ts` and `abi-shape.test.ts` hold every
// error vector to, run over `knownGapErrors`: revert payloads the contract
// really produces (encoded by its own `SolError` impl) whose error item the
// `.sol`-derived ABI does not carry. Each case fails the way a viem user
// experiences the gap — `decodeErrorResult` throws
// `AbiErrorSignatureNotFoundError` on the contract's actual revert bytes —
// rather than as a shape check on the JSON.
//
// When the emitter closes the gap, the guard in `abi-shape.test.ts` fails the
// blocking suite until the vector is moved from `knownGapErrors` to `errors`
// in corpus.rs. The assertions are shared, so the move is the whole promotion.

describe('revert payloads the .sol-path ABI cannot decode yet', () => {
  for (const contract of fixtures.contracts) {
    const gaps = contract.knownGapErrors ?? []
    if (gaps.length === 0) continue
    const abi = loadAbi(contract.abiFile)

    describe(contract.name, () => {
      it.each(gaps)('signature $id', (fixture) => expectErrorSignatureAgrees(abi, fixture))
      it.each(gaps)('decode $id', (fixture) => expectErrorDecodes(abi, fixture))
      it.each(gaps)('encode $id', (fixture) => expectErrorEncodes(abi, fixture))
    })
  }
})
