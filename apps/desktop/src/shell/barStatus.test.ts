import { describe, expect, it } from "vitest";
import type { Job, Status } from "@/app/types";
import { barStatus, runCounter } from "./barStatus";

function job(status: Status, startedAt: number | null = null): Job {
  return {
    id: 1,
    fileName: "a.pdf",
    sourcePath: "/a.pdf",
    jobType: "convert",
    service: "datalab",
    status,
    progressNote: "",
    route: null,
    reasonCodes: [],
    warnings: [],
    failure: null,
    outputPath: null,
    error: null,
    createdAt: 0,
    startedAt,
  };
}

const idle = { view: "run", jobs: [], documentName: null, workspaceName: null } as const;

describe("barStatus", () => {
  it("names the surface when nothing is running", () => {
    expect(barStatus(idle)).toEqual({ kind: "label", text: "Run", variant: "view" });
    expect(barStatus({ ...idle, view: "history" })).toEqual({
      kind: "label",
      text: "History",
      variant: "view",
    });
    expect(barStatus({ ...idle, view: "settings" })).toEqual({
      kind: "label",
      text: "Settings",
      variant: "view",
    });
  });

  it("names the workspace in the library, and keeps its case", () => {
    expect(barStatus({ ...idle, view: "library", workspaceName: "Tool-Kit" })).toEqual({
      kind: "label",
      text: "Tool-Kit",
      variant: "workspace",
    });
  });

  it("names the open document over the launcher", () => {
    expect(barStatus({ ...idle, documentName: "Resume.md" })).toEqual({
      kind: "label",
      text: "Resume.md",
      variant: "document",
    });
  });

  it("a live run outranks the surface name", () => {
    const status = barStatus({
      ...idle,
      view: "settings",
      documentName: "Resume.md",
      jobs: [job("done"), job("processing", 100)],
    });
    expect(status).toEqual({ kind: "run", done: 1, total: 2, since: 100 });
  });

  it("times the run from its earliest start, not the first row", () => {
    const status = barStatus({ ...idle, jobs: [job("done", 500), job("working", 200)] });
    expect(status).toMatchObject({ since: 200 });
  });

  it("survives a run whose jobs have not started yet", () => {
    expect(barStatus({ ...idle, jobs: [job("queued")] })).toEqual({
      kind: "run",
      done: 0,
      total: 1,
      since: null,
    });
  });

  it("returns to the surface name once every job is terminal", () => {
    expect(barStatus({ ...idle, jobs: [job("done"), job("failed")] })).toEqual({
      kind: "label",
      text: "Run",
      variant: "view",
    });
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
