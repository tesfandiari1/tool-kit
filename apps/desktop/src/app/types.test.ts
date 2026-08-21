import { describe, expect, it } from "vitest";
import { latestJobFor, type Job, type Status } from "./types";

function job(id: number, sourcePath: string, status: Status): Job {
  return {
    id,
    fileName: sourcePath,
    sourcePath,
    jobType: "convert",
    service: "Datalab",
    status,
    progressNote: "",
    route: null,
    reasonCodes: [],
    warnings: [],
    failure: null,
    outputPath: null,
    error: null,
    createdAt: 0,
    startedAt: null,
  };
}

describe("latestJobFor", () => {
  it("answers with nothing when the file has never been converted", () => {
    expect(latestJobFor([job(1, "/in/other.pdf", "done")], "/in/deck.pdf")).toBeNull();
  });

  /// The queue keeps every row a batch left behind, so a file retried from the
  /// library has two. Reading the older one leaves the tree showing a failure
  /// while the new conversion runs, with its Convert control still enabled.
  it("prefers the newest row when a file has been converted twice", () => {
    const jobs = [job(4, "/in/deck.pdf", "failed"), job(11, "/in/deck.pdf", "queued")];

    expect(latestJobFor(jobs, "/in/deck.pdf")?.status).toBe("queued");
  });

  it("reads the same way whatever order the rows arrived in", () => {
    const jobs = [job(11, "/in/deck.pdf", "working"), job(4, "/in/deck.pdf", "done")];

    expect(latestJobFor(jobs, "/in/deck.pdf")?.id).toBe(11);
  });
});
