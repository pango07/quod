import {test, expect} from 'vitest'
test('literal module is in the closed bundle', async () => {
  const loaded = await import('./literal-value.mjs')
  expect(loaded.value).toBe(42)
})
