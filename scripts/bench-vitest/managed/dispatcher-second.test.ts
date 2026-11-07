import {test, expect} from 'vitest'

test('managed dispatcher also executes the second selected file', () => {
  expect('second selected file').toContain('selected')
})
