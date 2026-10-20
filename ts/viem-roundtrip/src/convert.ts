import type { AbiParameter } from 'viem'
import { getAddress } from 'viem/utils'

/**
 * Turn a fixture JSON value into the value viem expects for `param`.
 *
 * The fixtures carry integers as decimal strings, because a JSON number loses
 * precision above 2^53 and a `uint256` vector would then pass for the wrong
 * reason. Everything else — hex for byte types, plain booleans and strings,
 * arrays for lists, objects for named tuples — is already in viem's shape, so
 * this walk exists mostly to place the integers.
 *
 * The integer target type is not uniform: abitype maps `uint8`…`uint48` to
 * `number` and `uint56`…`uint256` to `bigint`. Following that split keeps the
 * runtime vectors identical to what a TypeScript user would actually pass, and
 * keeps them consistent with what the decoders hand back.
 */
export function toViemValue(param: AbiParameter, value: unknown): unknown {
  const arrayMatch = /^(.*)\[(\d*)\]$/.exec(param.type)
  if (arrayMatch) {
    const element = { ...param, type: arrayMatch[1]! } as AbiParameter
    if (!Array.isArray(value)) {
      throw new Error(`expected an array for ${param.type}, got ${JSON.stringify(value)}`)
    }
    return value.map((item) => toViemValue(element, item))
  }

  if (param.type === 'tuple') {
    const components = ('components' in param ? param.components : []) as readonly AbiParameter[]
    // Unnamed components (an inline `.sol` tuple) are positional in viem;
    // named ones (a `struct`) are keyed.
    if (Array.isArray(value)) {
      return components.map((component, i) => toViemValue(component, value[i]))
    }
    const object = value as Record<string, unknown>
    const out: Record<string, unknown> = {}
    for (const component of components) {
      out[component.name!] = toViemValue(component, object[component.name!])
    }
    return out
  }

  // The fixtures store addresses lowercase, but viem's decoders return them
  // EIP-55 checksummed. Normalising here means one representation serves both
  // directions: viem's encoders accept either casing.
  if (param.type === 'address') {
    return getAddress(value as string)
  }

  const intMatch = /^u?int(\d*)$/.exec(param.type)
  if (intMatch) {
    const bits = intMatch[1] ? Number(intMatch[1]) : 256
    if (typeof value !== 'string') {
      throw new Error(`expected a decimal string for ${param.type}, got ${JSON.stringify(value)}`)
    }
    return bits <= 48 ? Number(value) : BigInt(value)
  }

  return value
}

/** Convert a whole parameter list. */
export function toViemValues(params: readonly AbiParameter[], values: readonly unknown[]): unknown[] {
  if (params.length !== values.length) {
    throw new Error(`expected ${params.length} values, got ${values.length}`)
  }
  return params.map((param, i) => toViemValue(param, values[i]))
}

/**
 * The value `decodeFunctionResult` should produce for the given outputs: the
 * bare value for one output, an array for several. viem's two shapes are not
 * normalised, because which one you get is part of what the suite checks.
 */
export function expectedResult(
  outputs: readonly AbiParameter[],
  json: unknown,
): unknown {
  if (outputs.length === 0) return undefined
  if (outputs.length === 1) return toViemValue(outputs[0]!, json)
  if (!Array.isArray(json)) {
    throw new Error(`expected an array for ${outputs.length} outputs, got ${JSON.stringify(json)}`)
  }
  return toViemValues(outputs, json)
}
