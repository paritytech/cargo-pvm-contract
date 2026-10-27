// Runs `test/known-gaps/` and checks that *exactly* the expected tests fail.
//
// The known-gaps directory holds real failing tests for emitter defects the
// suite found. Running it with `continue-on-error` would make a broken
// harness (an import error, a renamed helper, a fixture the loader cannot
// read) look identical to the 13 red-by-design cases, and a gap quietly
// closing would look like nothing at all. So instead the step is blocking and
// compares vitest's JSON report against `test/known-gaps/expected-failures.json`:
//
//   - an expected failure that now passes  => the gap closed: promote the test
//   - an expected failure that is missing  => the test was renamed or deleted
//   - an unexpected failure                => the harness is broken, not a gap
//
// Any of the three fails the step. To re-bless after promoting a test, edit the
// JSON list by hand — it is short, and a diff to it is part of the review.

import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = join(here, '..')
const expectedFile = join(root, 'test', 'known-gaps', 'expected-failures.json')

const tmp = mkdtempSync(join(tmpdir(), 'known-gaps-'))
const report = join(tmp, 'report.json')
try {
  const run = spawnSync(
    'pnpm',
    ['exec', 'vitest', 'run', '--config', 'vitest.known-gaps.config.ts', '--reporter=json', `--outputFile=${report}`],
    { cwd: root, stdio: ['ignore', 'inherit', 'inherit'] },
  )
  if (run.error) throw run.error

  let parsed
  try {
    parsed = JSON.parse(readFileSync(report, 'utf8'))
  } catch (error) {
    console.error(`known-gaps: vitest produced no JSON report (exit ${run.status}): ${error}`)
    process.exit(1)
  }

  const failed = new Set()
  const passed = new Set()
  for (const file of parsed.testResults ?? []) {
    const name = relative(root, file.name).replaceAll('\\', '/')
    if (file.status === 'failed' && (file.assertionResults ?? []).length === 0) {
      // The file itself failed to load — no tests ran.
      failed.add(`${name} :: <file failed to load>`)
      continue
    }
    for (const test of file.assertionResults ?? []) {
      const key = `${name} :: ${test.fullName}`
      if (test.status === 'failed') failed.add(key)
      else if (test.status === 'passed') passed.add(key)
    }
  }

  const expected = new Set(JSON.parse(readFileSync(expectedFile, 'utf8')))
  const problems = []
  for (const key of expected) {
    if (passed.has(key)) problems.push(`GAP CLOSED (promote to the blocking suite): ${key}`)
    else if (!failed.has(key)) problems.push(`MISSING (renamed or deleted?): ${key}`)
  }
  for (const key of failed) {
    if (!expected.has(key)) problems.push(`UNEXPECTED FAILURE (broken harness, not a gap): ${key}`)
  }

  if (problems.length > 0) {
    console.error('\nknown-gaps: the set of failing tests is not the expected one:')
    for (const p of problems) console.error(`  - ${p}`)
    console.error(`\nExpected list: ${relative(root, expectedFile)}`)
    process.exit(1)
  }
  console.log(`known-gaps: exactly the ${expected.size} expected tests fail, ${passed.size} pass.`)
} finally {
  rmSync(tmp, { recursive: true, force: true })
}
