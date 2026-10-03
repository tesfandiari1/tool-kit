import { describe, expect, it } from "vitest";
import { autodetectJob, canStartRun, planRun, runButtonLabel } from "./plan";
import { EMPTY_SCAN } from "@/app/types";

const scan = {
  ...EMPTY_SCAN,
  alreadyHereTranscribe: 3,
  reusableTranscribe: 2,
};

describe("autodetectJob", () => {
  const inputs = ["/drop"];

  it("picks the job with more matches", () => {
    expect(autodetectJob(inputs, null, { convert: 3, transcribe: 5 })?.jobType).toBe("transcribe");
    expect(autodetectJob(inputs, null, { convert: 5, transcribe: 3 })?.jobType).toBe("convert");
  });

  it("stays out of the way when the selection matches nothing", () => {
    expect(autodetectJob(inputs, null, { convert: 0, transcribe: 0 })).toBeNull();
  });

  it("answers the same selection only once", () => {
    const first = autodetectJob(inputs, null, { convert: 3, transcribe: 5 });
    expect(first).not.toBeNull();
    expect(autodetectJob(inputs, first?.selection ?? null, { convert: 3, transcribe: 5 })).toBeNull();
  });

  it("answers again after a new drop", () => {
    const first = autodetectJob(inputs, null, { convert: 3, transcribe: 5 });
    expect(autodetectJob(["/drop", "/other"], first?.selection ?? null, { convert: 5, transcribe: 3 })
      ?.jobType).toBe("convert");
  });

  it("survives the empty scan an unrelated refresh round-trips through", () => {
    // The bug this guards: an output-folder change zeroes the counts for one
    // commit, and the identical counts landing afterwards used to read as a new
    // drop and overwrite a job the user had clicked by hand.
    const answered = autodetectJob(inputs, null, { convert: 3, transcribe: 5 })?.selection ?? null;
    expect(autodetectJob(inputs, answered, { convert: 0, transcribe: 0 })).toBeNull();
    expect(autodetectJob(inputs, answered, { convert: 3, transcribe: 5 })).toBeNull();
  });
});

describe("planRun", () => {
  it("sends every match when skip-already-done is off", () => {
    expect(planRun("transcribe", 10, scan, false)).toEqual({
      skipping: 0,
      copying: 0,
      toRun: 10,
      colliding: 3,
    });
  });

  it("skips files already in the output folder before copying the rest", () => {
    expect(planRun("transcribe", 10, scan, true)).toEqual({
      skipping: 3,
      copying: 2,
      toRun: 5,
      colliding: 0,
    });
  });

  it("never skips or copies more files than the selection contains", () => {
    expect(planRun("transcribe", 2, scan, true)).toEqual({
      skipping: 2,
      copying: 0,
      toRun: 0,
      colliding: 0,
    });
  });

  it("never skips or copies a Convert file", () => {
    expect(planRun("convert", 6, scan, true)).toEqual({
      skipping: 0,
      copying: 0,
      toRun: 6,
      colliding: 0,
    });
  });

  it("treats a copy-only selection as free work, not a no-op", () => {
    const copyOnly = { ...EMPTY_SCAN, reusableTranscribe: 4 };
    expect(planRun("transcribe", 4, copyOnly, true)).toEqual({
      skipping: 0,
      copying: 4,
      toRun: 0,
      colliding: 0,
    });
  });

  it("caps numbered copies at the selection when skip is off", () => {
    expect(planRun("transcribe", 2, scan, false)).toEqual({
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

  it("allows a conversion run and a copy-only run", () => {
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
  it("names the work, then copies, and never claims a no-op is a run", () => {
    expect(runButtonLabel("Convert", 1, 0)).toBe("Convert 1 file");
    expect(runButtonLabel("Convert", 3, 2)).toBe("Convert 3 files");
    expect(runButtonLabel("Convert", 0, 2)).toBe("Copy 2 results");
    expect(runButtonLabel("Convert", 0, 0)).toBe("Run");
  });
});
