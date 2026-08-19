import { describe, expect, it } from "vitest";
import { pathCrumbs } from "./pathCrumbs";

describe("pathCrumbs", () => {
  it("keeps a short path whole", () => {
    expect(pathCrumbs("~/Desktop/a.md")).toEqual(["~", "Desktop", "a.md"]);
  });

  it("marks an absolute path with a root crumb", () => {
    expect(pathCrumbs("/Users/me/a.md")).toEqual(["/", "Users", "me", "a.md"]);
  });

  it("elides the middle, never the file", () => {
    const crumbs = pathCrumbs("~/Documents/Work/2026/Q3/report.md");
    expect(crumbs).toEqual(["~", "…", "Q3", "report.md"]);
    expect(crumbs[crumbs.length - 1]).toBe("report.md");
  });

  it("holds its crumb count once eliding starts, so the line cannot grow", () => {
    const deep = pathCrumbs("/a/b/c/d/e/f/g/h.md");
    expect(deep).toHaveLength(4);
    expect(pathCrumbs("/a/b/c/d/e.md")).toHaveLength(4);
  });

  it("honours a wider budget", () => {
    expect(pathCrumbs("~/Documents/Work/2026/Q3/report.md", 6)).toEqual([
      "~",
      "Documents",
      "Work",
      "2026",
      "Q3",
      "report.md",
    ]);
  });

  it("survives a bare file name and a trailing slash", () => {
    expect(pathCrumbs("a.md")).toEqual(["a.md"]);
    expect(pathCrumbs("~/Desktop/")).toEqual(["~", "Desktop"]);
  });
});
