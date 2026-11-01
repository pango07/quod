import { describe, expect, it } from "vitest";
import { joinCursor, splitCursor } from "./cursor";
import { mergeAsync } from "./merge";

describe("merged stream cursors", () => {
  it("joins and splits the GitHub and Quod positions", () => {
    expect(joinCursor("gh-12", null)).toBe("gh-12");
    expect(joinCursor("gh-12", "1793564360732-3658-3")).toBe("gh-12,1793564360732-3658-3");
    expect(joinCursor(null, "q")).toBe("q");
    expect(splitCursor("gh-12,1793564360732-3658-3")).toEqual({ gh: "gh-12", quod: "1793564360732-3658-3" });
    expect(splitCursor("gh-12")).toEqual({ gh: "gh-12", quod: null });
    // A single-source id (the simulator's, or quod-server's) passes through as Quod's.
    expect(splitCursor("1793564360732-3658-3")).toEqual({ gh: null, quod: "1793564360732-3658-3" });
    expect(splitCursor(null)).toEqual({ gh: null, quod: null });
    expect(splitCursor("gh-abc,x").gh).toBeNull();
  });
});

describe("mergeAsync", () => {
  it("interleaves sources in arrival order and stops on abort", async () => {
    const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
    async function* slow() {
      await wait(30);
      yield "b";
    }
    async function* fast() {
      yield "a";
      await wait(60);
      yield "c";
    }
    const ac = new AbortController();
    const out: string[] = [];
    for await (const { value } of mergeAsync([slow(), fast()], ac.signal)) out.push(value);
    expect(out).toEqual(["a", "b", "c"]);
  });
});
