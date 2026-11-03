import { expect, test } from 'vitest'
import { describeAdd } from '../src/math'

// A test name built from a function's source: changing that function's
// body must re-run this file (toString is watched).
test.each([1])(`${describeAdd} %s`, (n) => {
  expect(n).toBe(1)
})
