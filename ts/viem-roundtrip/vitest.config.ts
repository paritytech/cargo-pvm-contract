import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    // `test/types.test-d.ts` is deliberately excluded: `expectTypeOf` and
    // `@ts-expect-error` are compile-time assertions, and `pnpm typecheck`
    // (plain `tsc --noEmit` over `test/`) is what enforces them. Running them
    // through vitest as well would only add a second, slower mechanism for the
    // same checks.
    include: ['test/**/*.test.ts'],
  },
})
