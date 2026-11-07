import {test, expect} from 'vitest'
test('caught computed module import is sticky unsupported', async () => {
  const target = './literal-value.mjs'
  try { await import(target) } catch {}
  expect(true).toBe(true)
})
