import { describe, expect, it } from 'vitest'
import type { Abi, AbiEvent, AbiParameter } from 'viem'
import { decodeEventLog, encodeEventTopics, formatAbiItem } from 'viem/utils'

import { toViemValue } from '../src/convert.js'
import { fixtures, loadAbi } from '../src/fixtures.js'

describe('logs round-trip through viem', () => {
  for (const contract of fixtures.contracts) {
    if (contract.events.length === 0) continue
    const abi = loadAbi(contract.abiFile)

    describe(contract.name, () => {
      it.each(contract.events)('topics $id', (fixture) => {
        const event = findEvent(abi, fixture.signature)
        const args = convertArgs(event.inputs, fixture.args)

        if (hasCompositeIndexedParam(event)) {
          // viem refuses to hash an indexed tuple or array into a topic — it
          // cannot build a filter for one. The ABI itself is legal (solc emits
          // the same shape), so this pins the boundary rather than a defect:
          // such an event can be decoded but not filtered.
          expect(() => encodeEventTopics({ abi, eventName: event.name, args } as never)).toThrow(
            /not supported/i,
          )
          return
        }

        expect(encodeEventTopics({ abi, eventName: event.name, args } as never)).toEqual(
          fixture.topics,
        )
      })

      it.each(contract.events)('decode $id', (fixture) => {
        const event = findEvent(abi, fixture.signature)

        if (event.anonymous) {
          // An anonymous event has no signature topic, so viem has nothing to
          // look the ABI item up by and cannot decode the log at all.
          expect(() =>
            decodeEventLog({ abi, data: fixture.data, topics: fixture.topics as never }),
          ).toThrow()
          return
        }

        const decoded = decodeEventLog({
          abi,
          data: fixture.data,
          topics: fixture.topics as never,
        })
        expect(decoded.eventName).toBe(event.name)
        expect(decoded.args).toEqual(convertArgs(event.inputs, fixture.decoded))
      })
    })
  }
})

describe('indexed dynamic fields hash into their topic', () => {
  const abi = loadAbi('abi/abi-surface.abi.json')
  const fixture = fixtures.contracts
    .find((c) => c.name === 'abi-surface')!
    .events.find((e) => e.id === 'IndexedDynamic')!

  it('the topic is keccak256 of the raw bytes, not the value', () => {
    // The value is unrecoverable from the log, so `decodeEventLog` hands back
    // the 32-byte hash and only the non-indexed field survives as a value.
    const decoded = decodeEventLog({ abi, data: fixture.data, topics: fixture.topics as never })
    const args = decoded.args as unknown as Record<string, unknown>
    expect(args.name).toBe(fixture.decoded.name)
    expect(args.name).not.toBe('alice')
    expect(String(args.name)).toMatch(/^0x[0-9a-f]{64}$/)
    expect(args.note).toBe('transfer approved')
  })

  it('viem recomputes the same topic from the value', () => {
    const event = findEvent(abi, fixture.signature)
    const topics = encodeEventTopics({
      abi,
      eventName: event.name,
      args: convertArgs(event.inputs, fixture.args),
    } as never)
    expect(topics).toEqual(fixture.topics)
  })
})

describe('anonymous events carry no signature topic', () => {
  const abi = loadAbi('abi/abi-surface.abi.json')
  const fixture = fixtures.contracts
    .find((c) => c.name === 'abi-surface')!
    .events.find((e) => e.id === 'AnonymousPing')!

  it('topic count equals the indexed-field count', () => {
    const event = findEvent(abi, fixture.signature)
    expect(event.anonymous).toBe(true)
    const indexed = event.inputs.filter((input) => input.indexed)
    expect(fixture.topics).toHaveLength(indexed.length)
    expect(fixture.topics[0]).not.toBe(formatAbiItem(event))
  })

  it('viem produces the same topics', () => {
    const event = findEvent(abi, fixture.signature)
    expect(
      encodeEventTopics({
        abi,
        eventName: event.name,
        args: convertArgs(event.inputs, fixture.args),
      } as never),
    ).toEqual(fixture.topics)
  })
})

describe('event parameter names follow the ABI, not the Rust field names', () => {
  it('a .sol-derived event uses the Solidity spelling', () => {
    const abi = loadAbi('abi/events.abi.json')
    const fixture = fixtures.contracts.find((c) => c.name === 'events')!.events[0]!
    const decoded = decodeEventLog({ abi, data: fixture.data, topics: fixture.topics as never })
    expect(Object.keys(decoded.args as object)).toEqual(['who', 'oldValue', 'newValue'])
  })

  it('a Rust-derived event keeps snake_case', () => {
    const abi = loadAbi('abi/abi-surface.abi.json')
    const fixture = fixtures.contracts
      .find((c) => c.name === 'abi-surface')!
      .events.find((e) => e.id === 'Indexed3')!
    const decoded = decodeEventLog({ abi, data: fixture.data, topics: fixture.topics as never })
    expect(Object.keys(decoded.args as object)).toEqual(['who', 'amount', 'tag', 'note'])
  })
})

/** Convert a fixture's `{name: value}` map using the matching ABI parameters. */
function convertArgs(
  inputs: readonly AbiParameter[],
  args: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const input of inputs) {
    const name = input.name!
    if (!(name in args)) throw new Error(`fixture is missing a value for "${name}"`)
    const value = args[name]
    // A hashed topic is already a 32-byte hex string; converting it as its
    // declared type would be wrong (and would throw for tuples).
    out[name] = isHashedTopic(input, value) ? value : toViemValue(input, value)
  }
  return out
}

/**
 * An indexed `string`, `bytes`, tuple or array is stored as a hash, so a value
 * that arrived as bare hex for such a parameter is already the topic.
 */
function isHashedTopic(param: AbiParameter, value: unknown): boolean {
  const indexed = 'indexed' in param && param.indexed
  if (!indexed) return false
  const hashes =
    param.type === 'string' ||
    param.type === 'bytes' ||
    param.type === 'tuple' ||
    /\[\d*\]$/.test(param.type)
  return hashes && typeof value === 'string' && /^0x[0-9a-f]{64}$/.test(value)
}

function hasCompositeIndexedParam(event: AbiEvent): boolean {
  return event.inputs.some(
    (input) =>
      'indexed' in input &&
      input.indexed &&
      (input.type === 'tuple' || /\[\d*\]$/.test(input.type)),
  )
}

function findEvent(abi: Abi, signature: string): AbiEvent {
  const matches = abi.filter(
    (item): item is AbiEvent => item.type === 'event' && formatAbiItem(item) === signature,
  )
  expect(matches.length, `${signature} is missing from the ABI`).toBe(1)
  return matches[0]!
}
