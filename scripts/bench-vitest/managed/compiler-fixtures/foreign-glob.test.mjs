import {test, expect} from 'vitest'

const modules = import.meta.glob('./foreign-modules/*.mjs')
test('caught foreign glob loaders remain sticky and never execute their targets', async () => {
  expect(Object.keys(modules).sort()).toEqual(['./foreign-modules/native.mjs', './foreign-modules/value.mjs'])
  for (const name of Object.keys(modules)) {
    for (let attempt = 0; attempt < 2; attempt++) {
      let error
      try { await modules[name]() } catch (value) { error = value }
      expect(error?.message).toBe('QUOD_MANAGED_UNSUPPORTED: lazy module ' + name)
    }
  }
  expect(globalThis.__quodClosedForeignGlobEvaluations).toBeUndefined()
})
