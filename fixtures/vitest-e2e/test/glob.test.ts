import { expect, test } from 'vitest'

// What vite's glob found is part of this module's code (convex-test's pattern).
const modules = import.meta.glob('../src/lib/*.ts')

test('the glob finds the lib modules', async () => {
  const names = Object.keys(modules)
  expect(names.length).toBeGreaterThanOrEqual(2)
  const m = (await modules['../src/lib/one.ts']()) as { one: number }
  expect(m.one).toBe(1)
})
