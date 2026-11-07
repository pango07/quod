import {test, expect} from 'vitest'
const modules = import.meta.glob('./glob-modules/*.mjs')
test('glob has a closed module-name set without loading modules', () => {
  expect(Object.keys(modules).length).toBe(1)
})
