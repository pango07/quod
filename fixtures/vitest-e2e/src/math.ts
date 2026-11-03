// quod vitest's end-to-end fixture (crates/quod-cli/tests/vitest_e2e.rs).
export function add(a: number, b: number): number {
  return a + b
}

export function sub(a: number, b: number): number {
  return a - b
}

export const describeAdd = (n: number) => `adds ${n}`
