import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    // Only the top level of `test/`:
    //
    // - `test/known-gaps/` holds assertions the emitter does not satisfy yet.
    //   They run through `vitest.known-gaps.config.ts` (`pnpm test:known-gaps`)
    //   as a separate, non-blocking CI step.
    // - `test/types.test-d.ts` holds compile-time assertions (`expectTypeOf`,
    //   `@ts-expect-error`), which `pnpm typecheck` (plain `tsc --noEmit`)
    //   enforces; running them through vitest too would only add a second,
    //   slower mechanism for the same checks.
    include: ['test/*.test.ts'],
  },
})
