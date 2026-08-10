import { describe, expect, it } from 'vitest'
import type { Abi, AbiParameter } from 'viem'
import { formatAbiItem, getAbiItem, toEventSelector, toFunctionSelector } from 'viem/utils'

import { allAbiFiles, fixtures, loadAbi } from '../src/fixtures.js'

const abiFiles = allAbiFiles()

type AbiErrorItem = Extract<Abi[number], { type: 'error' }>
type AbiEventItem = Extract<Abi[number], { type: 'event' }>

describe('every emitted ABI is structurally valid for viem', () => {
  it.each(abiFiles)('$name', ({ abi }) => {
    expect(abi.length).toBeGreaterThan(0)

    for (const item of abi) {
      expect(item.type).toBeOneOf(['constructor', 'function', 'error', 'event', 'receive', 'fallback'])

      // viem reads `stateMutability` to decide whether a call may carry value.
      // The Rust serde model makes the field optional and skips it when absent,
      // so a codegen path that forgot to set it would silently make every
      // method look non-payable. Assert it is always there.
      if (item.type === 'function' || item.type === 'constructor' || item.type === 'receive') {
        expect(item, `${describeItem(item)} must declare stateMutability`).toHaveProperty(
          'stateMutability',
        )
        expect(['pure', 'view', 'nonpayable', 'payable']).toContain(item.stateMutability)
      }

      // `inputs`/`outputs` must be present as arrays even when empty; viem
      // indexes into them unconditionally.
      if (item.type === 'function') {
        expect(Array.isArray(item.inputs), `${item.name}.inputs must be an array`).toBe(true)
        expect(Array.isArray(item.outputs), `${item.name}.outputs must be an array`).toBe(true)
      }
      if (item.type === 'constructor' || item.type === 'error') {
        expect(Array.isArray(item.inputs)).toBe(true)
      }

      if (item.type === 'event') {
        expect(item).toHaveProperty('anonymous')
        for (const input of item.inputs) {
          expect(input, `${item.name}.${input.name} must declare indexed`).toHaveProperty('indexed')
        }
      }

      for (const param of parametersOf(item)) {
        assertTupleComponents(param, describeItem(item))
      }
    }
  })
})

describe('a `receive` entry is emitted for contracts with a receive handler', () => {
  it.each(['receive', 'abi-surface'])('%s', (name) => {
    const abi = loadAbi(`abi/${name}.abi.json`)
    const entry = abi.find((item) => item.type === 'receive')
    expect(entry).toEqual({ type: 'receive', stateMutability: 'payable' })
  })
})

describe('fixture signatures agree with the emitted ABI', () => {
  for (const contract of fixtures.contracts) {
    const abi = loadAbi(contract.abiFile)

    describe(contract.name, () => {
      // The strongest single assertion in the suite: viem derives the canonical
      // signature from the ABI item (expanding tuples through `components`),
      // hashes it, and the result has to equal what `const_selector` produced on
      // the Rust side. Any disagreement about camelCasing, tuple flattening or
      // type spelling shows up here rather than as an opaque byte diff.
      it.each(contract.functions)('function $id', (fixture) => {
        const item = findFunction(abi, fixture.functionName, fixture.signature)
        expect(formatAbiItem(item)).toBe(fixture.signature)
        expect(toFunctionSelector(formatAbiItem(item))).toBe(fixture.selector)
        expect(fixture.calldata.slice(0, 10)).toBe(fixture.selector)
      })

      it.each(contract.errors)('error $id', (fixture) => {
        const item = abi.find(
          (candidate) =>
            candidate.type === 'error' && formatAbiItem(candidate) === fixture.signature,
        )
        expect(item, `${fixture.signature} is missing from ${contract.abiFile}`).toBeDefined()
        expect((item as AbiErrorItem).name).toBe(fixture.errorName)
        // `formatAbiItem` is what viem's own `decodeErrorResult` hashes;
        // passing the item straight to `toFunctionSelector` would fold the
        // `error` keyword into the hash and give a different answer.
        expect(toFunctionSelector(formatAbiItem(item!))).toBe(fixture.selector)
        // For an error enum the enum's own selector is zeroed, so the leading
        // four bytes of the payload must be the held variant's.
        expect(fixture.data.slice(0, 10)).toBe(fixture.selector)
      })

      it.each(contract.events)('event $id', (fixture) => {
        const item = abi.find(
          (candidate) =>
            candidate.type === 'event' && formatAbiItem(candidate) === fixture.signature,
        )
        expect(item, `${fixture.signature} is missing from ${contract.abiFile}`).toBeDefined()
        const event = item as AbiEventItem
        expect(event.name).toBe(fixture.eventName)
        if (event.anonymous) {
          // No signature topic, so `topics` holds only indexed values.
          expect(fixture.topics.length).toBe(
            event.inputs.filter((input) => input.indexed).length,
          )
        } else {
          expect(fixture.topics[0]).toBe(toEventSelector(formatAbiItem(event)))
        }
      })
    })
  }
})

describe('framework errors are decodable from every ABI', () => {
  const expected = [
    'InvalidCalldata()',
    'CalldataTooLarge()',
    'NoSelector()',
    'UnknownSelector()',
    'NonPayableValueReceived()',
  ]

  it.each(abiFiles)('$name', ({ abi }) => {
    const signatures = abi
      .filter((item) => item.type === 'error')
      .map((item) => formatAbiItem(item))
    for (const signature of expected) {
      expect(signatures).toContain(signature)
    }
  })
})

// Items an ABI declares that no fixture exercises, with the reason. The gate
// below asserts this list is exactly right, so adding a contract method without a
// vector fails rather than quietly reducing coverage.
const UNEXERCISED_BY_DESIGN: Record<string, string[]> = {
  // `uint24`/`uint40`/`int72`/`uint200` and a *fixed* `uint8[N]` have no Rust
  // spelling — `[u8; N]` is `bytesN` and `Vec<u8>` is `uint8[]` — so no vector
  // can be produced for them. The parser must still map them, which is why they
  // stay in the interface.
  'sol-type-surface': ['oddWidths(uint24,uint40,int72)', 'byteArray(uint8[2])'],
  // Every signature here contains a type the emitter cannot express, so none of
  // them can be encoded at all. See abi-completeness.test.ts. `totalSupply` comes
  // from the `IToken` helper interface in the same file, which the parser
  // flattens in alongside the interface under test.
  'sol-reference-types': [
    'setToken(IToken)',
    'setTokens(IToken[])',
    'addNode((uint256,Node[]))',
    'totalSupply()',
  ],
}

describe('every ABI item is exercised by a fixture', () => {
  const FRAMEWORK_ERRORS = new Set([
    'InvalidCalldata()',
    'CalldataTooLarge()',
    'NoSelector()',
    'UnknownSelector()',
    'NonPayableValueReceived()',
  ])

  it.each(abiFiles)('$name', ({ name, abi, abiFile }) => {
    const contract = fixtures.contracts.find((c) => c.abiFile === abiFile)
    const exercised = new Set([
      ...(contract?.functions ?? []).map((f) => f.signature),
      ...(contract?.errors ?? []).map((e) => e.signature),
      ...(contract?.events ?? []).map((e) => e.signature),
    ])

    const unexercised = abi
      .filter((item) => item.type === 'function' || item.type === 'error' || item.type === 'event')
      .map((item) => formatAbiItem(item))
      .filter((signature) => !FRAMEWORK_ERRORS.has(signature) && !exercised.has(signature))

    expect(unexercised.sort()).toEqual((UNEXERCISED_BY_DESIGN[name] ?? []).sort())
  })
})

describe('every parameter type is a valid ABI type', () => {
  // viem parses an ABI lazily: an unrepresentable `type` string survives import
  // and only blows up at the first encode, decode or selector computation. This
  // walks every parameter of every item so such a type is caught up front,
  // whichever emitter produced it.
  it.each(abiFiles)('$name', ({ name, abi }) => {
    const invalid: string[] = []
    for (const item of abi) {
      for (const param of parametersOf(item)) {
        collectInvalidTypes(param, describeItem(item), invalid)
      }
    }
    if (name === 'sol-reference-types') {
      // This fixture exists precisely because the emitter produces invalid
      // types for it; the failure is asserted with its full explanation in
      // abi-completeness.test.ts rather than reported twice.
      expect(invalid.length).toBeGreaterThan(0)
      return
    }
    expect(invalid).toEqual([])
  })
})

/** `uint256`, `bytes4`, `string`, `tuple[2][]`, … — the ABI type grammar. */
const ELEMENTARY = /^(?:address|bool|string|bytes|function|tuple|(?:u?int(?:8|16|24|32|40|48|56|64|72|80|88|96|104|112|120|128|136|144|152|160|168|176|184|192|200|208|216|224|232|240|248|256))|bytes(?:[1-9]|1\d|2\d|3[0-2]))$/

function collectInvalidTypes(param: AbiParameter, context: string, out: string[]) {
  const base = param.type.replace(/(\[\d*\])+$/, '')
  const suffix = param.type.slice(base.length)
  if (!ELEMENTARY.test(base) || !/^(\[\d*\])*$/.test(suffix)) {
    out.push(`${context}: parameter "${param.name}" has type "${param.type}"`)
  }
  if (base === 'tuple' && 'components' in param) {
    for (const component of (param.components ?? []) as readonly AbiParameter[]) {
      collectInvalidTypes(component, context, out)
    }
  }
}

function parametersOf(item: Abi[number]): AbiParameter[] {
  const params: AbiParameter[] = []
  if ('inputs' in item && item.inputs) params.push(...(item.inputs as AbiParameter[]))
  if ('outputs' in item && item.outputs) params.push(...(item.outputs as AbiParameter[]))
  return params
}

/**
 * viem cannot encode or decode a `tuple` parameter without `components`, and it
 * cannot resolve a canonical signature for one either — the ABI would parse and
 * then fail at the first call.
 */
function assertTupleComponents(param: AbiParameter, context: string) {
  if (param.type.startsWith('tuple')) {
    expect(
      'components' in param && Array.isArray(param.components) && param.components.length > 0,
      `${context}: ${param.type} parameter "${param.name}" has no components`,
    ).toBe(true)
    for (const component of (param as { components: readonly AbiParameter[] }).components) {
      assertTupleComponents(component, context)
    }
  }
}

function describeItem(item: Abi[number]): string {
  return 'name' in item && item.name ? `${item.type} ${item.name}` : item.type
}

/**
 * Resolve an ABI function by name, disambiguating overloads by signature. viem's
 * `getAbiItem` picks by argument shape, which is not available here.
 */
function findFunction(abi: Abi, name: string, signature: string) {
  const matches = abi.filter(
    (item) => item.type === 'function' && item.name === name && formatAbiItem(item) === signature,
  )
  expect(matches.length, `${signature} is missing from the ABI`).toBe(1)
  return matches[0] as Extract<Abi[number], { type: 'function' }>
}

// Referenced so an unused-import lint cannot remove the ABI-item lookup viem
// users rely on; also a cheap sanity check that it agrees with `findFunction`.
describe('getAbiItem resolves non-overloaded fixture functions', () => {
  it('flipper.get', () => {
    const abi = loadAbi('abi/flipper.abi.json')
    expect(getAbiItem({ abi, name: 'get' })).toMatchObject({ name: 'get', type: 'function' })
  })
})
