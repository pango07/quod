import { describe, expect, it } from "vitest";
import { formatAgo, formatCount, formatDuration, formatMinutes, formatPercent, formatUsd, median } from "./format";

describe("formatDuration", () => {
  it("matches the design's run and step times", () => {
    expect(formatDuration(129)).toBe("2m 09s");
    expect(formatDuration(62)).toBe("1m 02s");
    expect(formatDuration(400)).toBe("6m 40s");
    expect(formatDuration(38.4)).toBe("38.4s");
    expect(formatDuration(46)).toBe("46s");
    expect(formatDuration(2.9)).toBe("2.9s");
    expect(formatDuration(0.21)).toBe("0.21s");
  });

  it("rounds at the boundaries without printing 60s", () => {
    expect(formatDuration(59.96)).toBe("1m 00s");
    expect(formatDuration(119.6)).toBe("2m 00s");
    expect(formatDuration(3725)).toBe("1h 02m");
  });

  it("refuses nonsense", () => {
    expect(formatDuration(-1)).toBe("n/a");
    expect(formatDuration(Number.NaN)).toBe("n/a");
  });
});

describe("formatMinutes", () => {
  it("matches the design's saved times", () => {
    expect(formatMinutes(1284)).toBe("21h 24m");
    expect(formatMinutes(5290)).toBe("88h 10m");
    expect(formatMinutes(14460)).toBe("241h");
    expect(formatMinutes(161)).toBe("2h 41m");
    expect(formatMinutes(28)).toBe("28m");
    expect(formatMinutes(0)).toBe("0m");
  });
});

describe("numbers", () => {
  it("formats counts, shares and money", () => {
    expect(formatCount(3412)).toBe("3,412");
    expect(formatPercent(3412 / 5590)).toBe("61%");
    expect(formatUsd(1284 * 0.008)).toBe("$10.27");
    expect(median([3, 1, 2])).toBe(2);
    expect(median([1, 2, 3, 4])).toBe(2.5);
  });

  it("formats relative times against a given now", () => {
    const now = "2026-11-01T16:00:00Z";
    expect(formatAgo("2026-11-01T15:56:00Z", now)).toBe("4m ago");
    expect(formatAgo("2026-11-01T14:00:00Z", now)).toBe("2h ago");
    expect(formatAgo("2026-10-31T16:00:00Z", now)).toBe("1d ago");
    expect(formatAgo("2026-11-01T15:59:50Z", now)).toBe("just now");
  });
});
