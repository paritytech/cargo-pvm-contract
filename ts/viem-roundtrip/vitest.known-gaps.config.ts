import { defineConfig } from 'vitest/config'

// The assertions in `test/known-gaps/` are expected to fail until the emitter
// catches up (see the comment at the top of each file there). They get their
// own config so `pnpm test` stays a blocking green gate while `pnpm
// test:known-gaps` keeps the gap visible in CI without blocking anything.
export default defineConfig({
  test: {
    include: ['test/known-gaps/*.test.ts'],
  },
})
