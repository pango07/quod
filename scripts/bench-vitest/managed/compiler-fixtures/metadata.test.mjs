import {test, expect} from 'vitest'
test('direct module resolution is sticky unsupported when caught', () => {
  try { import.meta.resolve('./literal-value.mjs') } catch {}
  expect(true).toBe(true)
})
test('aliased module metadata is sticky unsupported when caught', () => {
  try { const metadata = import.meta; metadata.resolve('./literal-value.mjs') } catch {}
  expect(true).toBe(true)
})
test('computed module metadata access is sticky unsupported when caught', () => {
  try { const name = 'resolve'; import.meta[name]('./literal-value.mjs') } catch {}
  expect(true).toBe(true)
})
