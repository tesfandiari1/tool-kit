import { describe, expect, it } from "vitest";
import {
  autodetectJob,
  canStartRun,
  effectiveSkipAlreadyDone,
  largeRunConfirmation,
  planRun,
  runButtonLabel,
  runServiceDescription,
} from "./plan";
import { EMPTY_SCAN } from "@/app/types";

const scan = {
  ...EMPTY_SCAN,
  alreadyHereConvert: 3,
  reusableConvert: 2,
  alreadyHereTranscribe: 1,
  reusableTranscribe: 4,
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
    expect(runButtonLabel("Convert", 0, 0)).toBe("Run");
  });
});

describe("effectiveSkipAlreadyDone", () => {
  it("disables reuse only for backend-routed conversion", () => {
    expect(effectiveSkipAlreadyDone("convert", "backend", true)).toBe(false);
    expect(effectiveSkipAlreadyDone("convert", "direct", true)).toBe(true);
    expect(effectiveSkipAlreadyDone("transcribe", "backend", true)).toBe(true);
    expect(effectiveSkipAlreadyDone("convert", "backend", false)).toBe(false);
  });
});

describe("largeRunConfirmation", () => {
  it("names provider credits for direct batches", () => {
    expect(
      largeRunConfirmation({
        totalFiles: 25,
        provider: "Datalab",
        backendFiles: 0,
        directFiles: 25,
        highAccuracy: true,
        profile: "standard",
      }),
    ).toBe(
      "This will send 25 files to Datalab, with high-accuracy convert on (slower, more credits per page).\n\nEach file uses Datalab credits.",
    );
  });

  it("warns that standard backend-only batches may still use remote fallback credits", () => {
    expect(
      largeRunConfirmation({
        totalFiles: 25,
        provider: "Datalab",
        backendFiles: 25,
        directFiles: 0,
        highAccuracy: true,
        profile: "standard",
      }),
    ).toBe(
      "This will send 25 files to your conversion backend.\n\nFiles that require remote fallback may also use Datalab credits with high-accuracy convert on (slower, more credits per page).",
    );
  });

  it("states direct cost and possible fallback cost for a mixed standard batch", () => {
    expect(
      largeRunConfirmation({
        totalFiles: 25,
        provider: "Datalab",
        backendFiles: 18,
        directFiles: 7,
        highAccuracy: false,
        profile: "standard",
      }),
    ).toBe(
      "This will send 18 files to your conversion backend and 7 files to Datalab.\n\nThe 7 files routed directly to Datalab use provider credits. Backend files may also use Datalab credits if remote fallback is required.",
    );
  });

  it("promises no fallback credits only under Local only", () => {
    expect(
      largeRunConfirmation({
        totalFiles: 25,
        provider: "Datalab",
        backendFiles: 25,
        directFiles: 0,
        highAccuracy: true,
        profile: "local_only",
      }),
    ).toBe(
      "This will send 25 files to your conversion backend.\n\nLocal only forbids Datalab fallback, so no Datalab credits are planned.",
    );
  });
});

describe("runServiceDescription", () => {
  const describe = (jobType: "convert" | "transcribe", route: "direct" | "backend", profile: "standard" | "local_only") =>
    runServiceDescription({
      description: jobType === "convert" ? "Extract structured text" : "Transcribe audio",
      provider: jobType === "convert" ? "Datalab" : "Rev.ai",
      jobType,
      conversionRoute: route,
      profile,
    });

  it("keeps direct conversion and transcription on their named provider", () => {
    expect(describe("convert", "direct", "standard")).toBe(
      "Extract structured text, via Datalab",
    );
    expect(describe("transcribe", "backend", "local_only")).toBe(
      "Transcribe audio, via Rev.ai",
    );
  });

  it("names fallback policy for backend conversion", () => {
    expect(describe("convert", "backend", "standard")).toBe(
      "Extract structured text, via your conversion backend with Datalab fallback where required",
    );
    expect(describe("convert", "backend", "local_only")).toBe(
      "Extract structured text, via your conversion backend only",
    );
  });
});
