import {test, expect} from 'vitest'

const key = '__quodClosedSelfGlobEvaluations'
globalThis[key] = (globalThis[key] || 0) + 1
export const token = {}
export const evaluations = () => globalThis[key]
const modules = import.meta.glob('./*.test.mjs')

test('self glob preserves lazy asynchronous imports and the original namespace', async () => {
  expect(evaluations()).toBe(1)
  const first = modules['./self-glob.test.mjs']()
  const second = modules['./self-glob.test.mjs']()
  expect(first).not.toBe(second)
  let settled = false
  first.then(() => { settled = true })
  expect(settled).toBe(false)
  const [left, right] = await Promise.all([first, second])
  expect(left).toBe(right)
  expect(left).toBe(await import('./self-glob.test.mjs'))
  expect(left.token).toBe(token)
  expect(left.evaluations()).toBe(1)
})

test('foreign keys stay present without evaluating their modules', () => {
  expect(Object.hasOwn(modules, './foreign-glob.test.mjs')).toBe(true)
  expect(evaluations()).toBe(1)
  expect(globalThis.__quodClosedForeignGlobEvaluations).toBeUndefined()
})
