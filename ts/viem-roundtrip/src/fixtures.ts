import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { Abi, AbiParameter } from 'viem'

const here = dirname(fileURLToPath(import.meta.url))
const fixturesDir = join(here, '..', 'fixtures')

/** Mirrors `ParameterCase` in `crates/pvm-viem-roundtrip/src/lib.rs`. */
export type ParameterCase = {
  id: string
  types: AbiParameter[]
  values: unknown[]
  encoded: `0x${string}`
}

/** Mirrors `FunctionCase`. */
export type FunctionCase = {
  id: string
  functionName: string
  signature: string
  selector: `0x${string}`
  args: unknown[]
  calldata: `0x${string}`
  result?: unknown
  returndata?: `0x${string}`
}

/** Mirrors `ErrorCase`. */
export type ErrorCase = {
  id: string
  errorName: string
  signature: string
  selector: `0x${string}`
  args: unknown[]
  data: `0x${string}`
}

/** Mirrors `EventCase`. */
export type EventCase = {
  id: string
  eventName: string
  signature: string
  topics: `0x${string}`[]
  data: `0x${string}`
  args: Record<string, unknown>
  decoded: Record<string, unknown>
}

/** Mirrors `ContractFixture`. */
export type ContractFixture = {
  name: string
  abiFile: string
  wrapped: boolean
  functions: FunctionCase[]
  errors: ErrorCase[]
  events: EventCase[]
}

/** Mirrors `Fixtures`. */
export type Fixtures = {
  parameters: ParameterCase[]
  contracts: ContractFixture[]
}

export const fixtures: Fixtures = JSON.parse(
  readFileSync(join(fixturesDir, 'vectors.json'), 'utf8'),
)

/**
 * Read an ABI fixture straight off disk, unwrapping the
 * `{"abi":…,"storageLayout":…}` container the builder writes for contracts with
 * declared storage.
 *
 * The runtime suites read the JSON rather than importing the generated
 * `abis.ts`, so a mistake in the codegen script cannot make a runtime test pass
 * against something the Rust side never emitted. `abis.ts` exists for the type
 * tests, where a literal is the whole point.
 */
export function loadAbi(abiFile: string): Abi {
  const raw = JSON.parse(readFileSync(join(fixturesDir, abiFile), 'utf8'))
  const abi = Array.isArray(raw) ? raw : raw.abi
  if (!Array.isArray(abi)) {
    throw new Error(`${abiFile}: expected an ABI array or an object with an "abi" key`)
  }
  return abi as Abi
}

/** Every ABI fixture, whether or not any corpus case references it. */
export function allAbiFiles(): { name: string; abiFile: string; abi: Abi; wrapped: boolean }[] {
  const dir = join(fixturesDir, 'abi')
  return readdirSync(dir)
    .filter((f) => f.endsWith('.abi.json'))
    .sort()
    .map((f) => {
      const abiFile = `abi/${f}`
      const raw = JSON.parse(readFileSync(join(fixturesDir, abiFile), 'utf8'))
      return {
        name: f.replace(/\.abi\.json$/, ''),
        abiFile,
        abi: loadAbi(abiFile),
        wrapped: !Array.isArray(raw),
      }
    })
}
