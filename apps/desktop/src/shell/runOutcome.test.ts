import { describe, expect, it } from "vitest";
import type { Job, Status } from "@/app/types";
import { autoOpenTarget, newlyDone, terminalIds } from "./runOutcome";

function job(id: number, status: Status, outputPath: string | null = `/out/${String(id)}.md`): Job {
  return {
    id,
    fileName: `${String(id)}.pdf`,
    sourcePath: `/in/${String(id)}.pdf`,
    jobType: "convert",
    service: "datalab",
    status,
    progressNote: "",
    route: null,
    reasonCodes: [],
    warnings: [],
    failure: null,
    outputPath: status === "done" ? outputPath : null,
    error: null,
    createdAt: 0,
    startedAt: null,
  };
}

describe("terminalIds", () => {
  it("takes done and failed, and leaves the queue alone", () => {
    const before = terminalIds([job(1, "done"), job(2, "failed"), job(3, "queued"), job(4, "working")]);
    expect([...before].sort((a, b) => a - b)).toEqual([1, 2]);
  });

  it("is empty for a queue the run just cleared", () => {
    expect(terminalIds([]).size).toBe(0);
  });
});

describe("newlyDone", () => {
  it("names the file a fresh single-file run produced", () => {
    expect(newlyDone(new Set(), [job(1, "done")]).map((j) => j.id)).toEqual([1]);
  });

  it("ignores results that were already on the board", () => {
    // convert_one appends rather than clearing, so the whole list is 201 rows
    // and 200 of them were done before this file was ever queued.
    const before = new Set([1, 2, 3]);
    const jobs = [job(1, "done"), job(2, "done"), job(3, "done"), job(4, "done")];
    expect(newlyDone(before, jobs).map((j) => j.id)).toEqual([4]);
  });

  it("counts a retried row, because a retry takes it out of the snapshot", () => {
    // The row was failed at the end of the last run and is queued again by the
    // time `running` flips, so it is not in `before`.
    const before = terminalIds([job(1, "done"), job(2, "queued")]);
    expect(newlyDone(before, [job(1, "done"), job(2, "done")]).map((j) => j.id)).toEqual([2]);
  });

  it("is not fooled by a failure", () => {
    expect(newlyDone(new Set(), [job(1, "failed")])).toEqual([]);
  });
});

describe("autoOpenTarget", () => {
  it("opens the one file a single-file run produced", () => {
    expect(autoOpenTarget(new Set(), [job(1, "done")])?.id).toBe(1);
  });

  it("opens nothing for a batch", () => {
    expect(autoOpenTarget(new Set(), [job(1, "done"), job(2, "done")])).toBeNull();
  });

  it("opens nothing when the run produced no result", () => {
    expect(autoOpenTarget(new Set(), [job(1, "failed")])).toBeNull();
    expect(autoOpenTarget(new Set(), [])).toBeNull();
  });

  it("opens the survivor of a stopped run", () => {
    // Stop rewrites every unfinished row to failed, so one finished file out
    // of two is still one result the user paid for and can read.
    expect(autoOpenTarget(new Set(), [job(1, "done"), job(2, "failed")])?.id).toBe(1);
  });

  it("refuses a done job that wrote nowhere", () => {
    const wrote = { ...job(1, "done"), outputPath: null };
    expect(autoOpenTarget(new Set(), [wrote])).toBeNull();
  });
});
