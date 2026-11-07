import {test, expect} from 'vitest'
test('caught direct eval is sticky unsupported', () => {
  try { eval('1 + 1') } catch {}
  expect(true).toBe(true)
})
test('caught Function construction is sticky unsupported', () => {
  try { new Function('return 42')() } catch {}
  expect(true).toBe(true)
})
