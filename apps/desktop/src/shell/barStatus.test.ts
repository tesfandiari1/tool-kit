import { describe, expect, it } from "vitest";
import type { Job, Status } from "@/app/types";
import { barStatus, runCounter } from "./barStatus";

function job(status: Status, startedAt: number | null = null): Job {
  return {
    id: 1,
    fileName: "a.pdf",
    sourcePath: "/a.pdf",
    jobType: "convert",
    status,
    progressNote: "",
    route: null,
    reasonCodes: [],
    warnings: [],
    failure: null,
    outputPath: null,
    error: null,
    startedAt,
  };
}

describe("barStatus", () => {
  it("says nothing when nothing is running", () => {
    expect(barStatus([])).toBeNull();
  });

  it("counts the finished files against the whole run", () => {
    expect(barStatus([job("done"), job("processing", 100)])).toEqual({
      done: 1,
      total: 2,
      since: 100,
    });
  });

  it("times the run from its earliest start, not the first row", () => {
    expect(barStatus([job("done", 500), job("working", 200)])).toMatchObject({ since: 200 });
  });

  it("survives a run whose jobs have not started yet", () => {
    expect(barStatus([job("queued")])).toEqual({ done: 0, total: 1, since: null });
  });

  it("goes quiet once every job is terminal", () => {
    expect(barStatus([job("done"), job("failed")])).toBeNull();
  });
});

describe("runCounter", () => {
  it("holds its width as the counter passes ten, so the timer never shifts", () => {
    expect(runCounter(9, 12)).toHaveLength(runCounter(10, 12).length);
    expect(runCounter(9, 12)).toBe(" 9 / 12");
    expect(runCounter(10, 12)).toBe("10 / 12");
  });

  it("does not pad when it cannot help", () => {
    expect(runCounter(1, 3)).toBe("1 / 3");
  });
});
