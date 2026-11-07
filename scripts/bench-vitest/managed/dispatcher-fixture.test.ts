import {test, expect} from 'vitest'

test('managed dispatcher preserves the selected test outcome', async () => {
  const mode = process.env.QUOD_DISPATCHER_FIXTURE_MODE || 'pass'
  if (['unsupported-pass', 'unsupported-failure', 'guard-assertion'].includes(mode)) {
    let fetched = false
    try { fetched = await (await fetch('data:text/plain,dispatcher-fixture')).text() === 'dispatcher-fixture' } catch {}
    // This intentionally fails under a caught managed guard and passes in
    // ordinary Vitest. Unsupported execution must use the ordinary outcome.
    if (mode === 'guard-assertion') expect(fetched).toBe(true)
  }
  if (mode === 'failure' || mode === 'unsupported-failure') expect('failure').toBe('pass')
  expect(2 + 3).toBe(5)
  console.log('QUOD_DISPATCHER_FIXTURE_COMPLETED')
})
