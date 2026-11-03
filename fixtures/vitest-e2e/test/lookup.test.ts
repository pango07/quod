import { expect, test } from 'vitest'

// The same glob, read by key only: a new file under src/lib is not this
// file's input, unless it is a key the file looked up (nine.ts).
const modules = import.meta.glob('../src/lib/*.ts')
const nine = '../src/lib/nine.ts' in modules ? 'there' : 'absent'

test(`one loads; nine is ${nine}`, async () => {
  const m = (await modules['../src/lib/one.ts']()) as { one: number }
  expect(m.one).toBe(1)
})
