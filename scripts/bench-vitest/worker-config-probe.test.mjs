import assert from 'node:assert/strict'
import test from 'node:test'
import { createRequire } from 'node:module'
import { types, inspect } from 'node:util'
const { observeConfig } = createRequire(import.meta.url)('./worker-config-probe.cjs')

test('distinguishes unused seed, direct reads, descriptor reads and enumeration', () => {
  const p = observeConfig({ sequence: { shuffle: false, seed: 123 }, timeout: 1000 })
  assert.equal(p.value.sequence.shuffle, false)
  assert(!p.report().events.some(e => e.path.join('.') === 'sequence.seed'))
  assert.equal(p.value.sequence.seed, 123)
  assert.equal(Object.getOwnPropertyDescriptor(p.value.sequence, 'seed').value, 123)
  assert.deepEqual(Object.keys(p.value.sequence), ['shuffle', 'seed'])
  const events = p.report().events.filter(e => e.path.join('.') === 'sequence.seed')
  assert(events.some(e => e.operation === 'get'))
  assert(events.some(e => e.operation === 'descriptor'))
  assert(p.report().seedStacks.length > 0)
  assert.equal(p.report().replayEligible, false)
})

test('preserves tested alias, mutation and descriptor behavior and reports gaps', () => {
  const nested = { seed: 3 }, config = { a: nested, b: nested }
  const p = observeConfig(config)
  assert.equal(p.value.a, p.value.b)
  p.value.a.seed = 4
  assert.equal(config.a.seed, 4)
  delete p.value.b.seed
  assert(!('seed' in config.a))
  Object.defineProperty(config, 'fixed', { value: { secret: 1 }, configurable: false, writable: false })
  assert.equal(p.value.fixed, config.fixed)
  assert(p.report().gaps.includes('fixed object property'))
  assert.equal(p.report().replayEligible, false)
})

test('native inspection can bypass traps, so absence of reads is never replay authority', () => {
  const p = observeConfig({ sequence: { seed: 123 } })
  assert.equal(types.isProxy(p.value), true)
  assert.match(inspect(p.value), /seed: 123/)
  assert.equal(p.report().events.length, 0)
  assert.equal(p.report().nativeReadsExcluded, false)
  assert.equal(p.report().proxyObservable, true)
  assert.equal(p.report().replayEligible, false)
})
