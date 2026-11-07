import {test, expect, vi} from 'vitest'
import {AsyncLocalStorage} from 'node:async_hooks'

test('real spies retain reset and restore behavior', () => {
  const target = {value: () => 4}
  const spy = vi.spyOn(target, 'value').mockReturnValue(7)
  expect(target.value()).toBe(7)
  expect(spy).toHaveBeenCalledExactlyOnceWith()
  vi.restoreAllMocks()
  expect(target.value()).toBe(4)
  const mock = vi.fn().mockReturnValue(9)
  expect(mock()).toBe(9)
  vi.clearAllMocks()
  expect(mock).not.toHaveBeenCalled()
  expect(mock()).toBe(9)
  vi.resetAllMocks()
  expect(mock()).toBeUndefined()
})

test('actual async assertions and context propagation work', async () => {
  const storage = new AsyncLocalStorage()
  await expect(Promise.all([1, 2].map(value => storage.run(value, async () => {
    await new Promise(resolve => setTimeout(resolve, 1))
    return storage.getStore()
  })))).resolves.toEqual([1, 2])
  expect(storage.getStore()).toBeUndefined()
})

test('an assertion failure stays a failure', () => {
  expect(process.env.QUOD_FIXTURE_MODE).not.toBe('failure')
})

test('catching an unsupported operation cannot erase it', async () => {
  if (process.env.QUOD_FIXTURE_MODE === 'unsupported') {
    // Ordinary Vitest performs this harmless data-URL fetch; the managed
    // runtime rejects every fetch and retains the flag even when caught.
    try { await fetch('data:text/plain,fixture') } catch {}
  }
  expect(true).toBe(true)
})
