import {test, expect} from 'vitest'
import {OriginalClassName as FirstClass, originalFunctionName as firstFunction} from './name-collision-one.mjs'
import {OriginalClassName as SecondClass, originalFunctionName as secondFunction} from './name-collision-two.mjs'

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

test('bundled colliding classes and functions preserve their original names', async () => {
  const namespace = await modules['./self-glob.test.mjs']()
  expect(namespace.token).toBe(token)
  expect(FirstClass).not.toBe(SecondClass)
  expect(firstFunction).not.toBe(secondFunction)
  for (const Class of [FirstClass, SecondClass]) {
    expect(Class.name).toBe('OriginalClassName')
    expect(new Class().constructor.name).toBe('OriginalClassName')
  }
  for (const fn of [firstFunction, secondFunction]) expect(fn.name).toBe('originalFunctionName')
  expect(new FirstClass().value()).toBe(1)
  expect(new SecondClass().value()).toBe(2)
  expect(firstFunction()).toBe(1)
  expect(secondFunction()).toBe(2)
})
