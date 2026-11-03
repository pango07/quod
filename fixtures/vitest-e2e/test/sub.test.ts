import { expect, test } from 'vitest'
import { sub } from '../src/math'

test('sub', () => {
  expect(sub(3, 2)).toBe(1)
})
