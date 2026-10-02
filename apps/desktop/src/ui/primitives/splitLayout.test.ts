import { describe, expect, it } from "vitest";
import { paneLayout } from "./splitLayout";

describe("paneLayout", () => {
  it("keeps the saved start on the start pane", () => {
    expect(paneLayout({ end: 68.854, start: 31.146 }, 33)).toEqual({
      start: 31.146,
      end: 68.854,
    });
  });

  it("falls back to the default before the seam has been dragged", () => {
    expect(paneLayout(undefined, 33)).toEqual({ start: 33, end: 67 });
  });

  it("repairs a pair that no longer sums to 100", () => {
    // The library throws on a layout it cannot normalise, so a stale or
    // hand-edited settings.json must not be able to reach it intact.
    const repaired = paneLayout({ start: 40, end: 40 }, 33);
    expect(repaired.start + repaired.end).toBe(100);
    expect(repaired.start).toBe(40);
  });

  it("gives the document two thirds by default", () => {
    expect(paneLayout(undefined, 33).end).toBeGreaterThan(paneLayout(undefined, 33).start * 2);
  });
});
