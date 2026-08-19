import { describe, expect, it } from "vitest";
import { ZOOM } from "./geometry";
import { nextZoom, zoomLabel } from "./useZoom";

describe("nextZoom", () => {
  it("steps one rung of the configured size", () => {
    expect(nextZoom(ZOOM.default, 1)).toBe(1.1);
    expect(nextZoom(ZOOM.default, -1)).toBe(0.9);
  });

  it("returns to the exact starting factor after up then down", () => {
    // The reason the ladder exists: 1 + 0.1 - 0.1 is not 1 in binary floating
    // point, so `toBe` here is the assertion that matters, not `toBeCloseTo`.
    expect(nextZoom(nextZoom(ZOOM.default, 1), -1)).toBe(ZOOM.default);
    expect(nextZoom(nextZoom(ZOOM.default, -1), 1)).toBe(ZOOM.default);
  });

  it("walks the whole ladder and back without drifting", () => {
    let up: number = ZOOM.min;
    for (let i = 0; i < 100; i++) up = nextZoom(up, 1);
    expect(up).toBe(ZOOM.max);
    let down: number = up;
    for (let i = 0; i < 100; i++) down = nextZoom(down, -1);
    expect(down).toBe(ZOOM.min);
  });

  it("clamps at both ends instead of running off the ladder", () => {
    expect(nextZoom(ZOOM.max, 1)).toBe(ZOOM.max);
    expect(nextZoom(ZOOM.min, -1)).toBe(ZOOM.min);
  });

  it("pulls a factor from outside the range back onto it", () => {
    expect(nextZoom(4, 1)).toBe(ZOOM.max);
    expect(nextZoom(4, -1)).toBe(ZOOM.max);
    expect(nextZoom(0.1, -1)).toBe(ZOOM.min);
    expect(nextZoom(0.1, 1)).toBe(ZOOM.min);
  });

  it("snaps a factor between rungs to the next rung, not past it", () => {
    expect(nextZoom(1.15, 1)).toBe(1.2);
    expect(nextZoom(1.15, -1)).toBe(1.1);
  });

  it("resets from any factor", () => {
    expect(nextZoom(ZOOM.max, 0)).toBe(ZOOM.default);
    expect(nextZoom(ZOOM.min, 0)).toBe(ZOOM.default);
    expect(nextZoom(1.15, 0)).toBe(ZOOM.default);
  });
});

describe("zoomLabel", () => {
  it("reads as whole percent", () => {
    expect(zoomLabel(1.1)).toBe("110%");
    expect(zoomLabel(1.25)).toBe("125%");
    expect(zoomLabel(ZOOM.default)).toBe("100%");
    expect(zoomLabel(ZOOM.min)).toBe("50%");
  });
});
