import { describe, expect, it } from "vitest";
import { canStartRun, planRun, runButtonLabel } from "./plan";
import { EMPTY_SCAN } from "@/app/types";

const scan = {
  ...EMPTY_SCAN,
  alreadyHereConvert: 3,
  reusableConvert: 2,
  alreadyHereTranscribe: 1,
  reusableTranscribe: 4,
};

describe("planRun", () => {
  it("sends every match when skip-already-done is off", () => {
    expect(planRun("convert", 10, scan, false)).toEqual({
      skipping: 0,
      copying: 0,
      toRun: 10,
      colliding: 3,
    });
  });

  it("skips files already in the output folder before copying the rest", () => {
    expect(planRun("convert", 10, scan, true)).toEqual({
      skipping: 3,
      copying: 2,
      toRun: 5,
      colliding: 0,
    });
  });

  it("never skips or copies more files than the selection contains", () => {
    expect(planRun("convert", 2, scan, true)).toEqual({
      skipping: 2,
      copying: 0,
      toRun: 0,
      colliding: 0,
    });
  });

  it("uses the transcribe buckets when that job is selected", () => {
    expect(planRun("transcribe", 6, scan, true)).toEqual({
      skipping: 1,
      copying: 4,
      toRun: 1,
      colliding: 0,
    });
  });

  it("treats a copy-only selection as free work, not a no-op", () => {
    const copyOnly = { ...EMPTY_SCAN, reusableConvert: 4 };
    expect(planRun("convert", 4, copyOnly, true)).toEqual({
      skipping: 0,
      copying: 4,
      toRun: 0,
      colliding: 0,
    });
  });

  it("caps numbered copies at the selection when skip is off", () => {
    expect(planRun("convert", 2, scan, false)).toEqual({
      skipping: 0,
      copying: 0,
      toRun: 2,
      colliding: 2,
    });
  });
});

describe("canStartRun", () => {
  const ready = {
    hasInputs: true,
    hasOutput: true,
    hasKey: true,
    toRun: 1,
    copying: 0,
    running: false,
    starting: false,
  };

  it("allows a billable run and a copy-only run", () => {
    expect(canStartRun(ready)).toBe(true);
    expect(canStartRun({ ...ready, toRun: 0, copying: 2 })).toBe(true);
  });

  it("blocks when there is nothing to send or copy, or a run is in flight", () => {
    expect(canStartRun({ ...ready, toRun: 0, copying: 0 })).toBe(false);
    expect(canStartRun({ ...ready, hasKey: false })).toBe(false);
    expect(canStartRun({ ...ready, hasOutput: false })).toBe(false);
    expect(canStartRun({ ...ready, running: true })).toBe(false);
    expect(canStartRun({ ...ready, starting: true })).toBe(false);
  });
});

describe("runButtonLabel", () => {
  it("names the billable work, then copies, and never claims a no-op is a run", () => {
    expect(runButtonLabel("Convert", 1, 0)).toBe("Convert 1 file");
    expect(runButtonLabel("Convert", 3, 2)).toBe("Convert 3 files");
    expect(runButtonLabel("Convert", 0, 2)).toBe("Copy 2 results");
    expect(runButtonLabel("Convert", 0, 0)).toBe("Run pipeline");
  });
});
