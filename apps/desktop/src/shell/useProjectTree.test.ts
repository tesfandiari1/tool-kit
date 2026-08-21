import { describe, expect, it } from "vitest";
import type { DirListing, FileRow } from "@/app/types";
import { childDirs, climbTarget, listFailure } from "./useProjectTree";

function row(rel: string, isDir: boolean): FileRow {
  return {
    path: `/ws/${rel}`,
    rel,
    name: rel.split("/").pop() ?? rel,
    isDir,
    ext: isDir ? "" : "pdf",
    mediaType: "",
    size: 0,
    modifiedMs: 0,
    job: null,
    resultPath: null,
    resultName: null,
    openable: false,
    resultOpenable: false,
  };
}

function listing(entries: FileRow[]): DirListing {
  return { modifiedMs: 0, entries, truncated: 0, pending: 0 };
}

describe("listFailure", () => {
  /// The whole point of the typed error: only a folder that is really gone may
  /// cost the user the folders they had open. A volume that went to sleep must
  /// not empty the persisted expansion on a background read.
  it("reads a missing folder as gone and everything else as still there", () => {
    expect(listFailure({ gone: true, message: "No such file or directory" })).toEqual({
      gone: true,
      message: "No such file or directory",
    });
    expect(listFailure({ gone: false, message: "Input/output error" }).gone).toBe(false);
  });

  it("treats a rejection it cannot read as still there", () => {
    expect(listFailure("boom")).toEqual({ gone: false, message: "boom" });
    expect(listFailure(new Error("offline")).gone).toBe(false);
  });
});

describe("climbTarget", () => {
  /// A row that unmounts holding focus drops it on <body>, which ends keyboard
  /// navigation silently. The selection has to be out of the subtree first.
  it("climbs to the parent when the selection is the doomed row", () => {
    expect(climbTarget("Inbox/reports", "Inbox/reports")).toBe("Inbox");
  });

  it("climbs from any depth inside the doomed row", () => {
    expect(climbTarget("Inbox/reports/q3/deck.pdf", "Inbox/reports")).toBe("Inbox");
  });

  it("leaves a selection outside the doomed row alone", () => {
    expect(climbTarget("Inbox/notes.md", "Inbox/reports")).toBeNull();
    expect(climbTarget("Inbox/reportsmore/a.md", "Inbox/reports")).toBeNull();
    expect(climbTarget(null, "Inbox/reports")).toBeNull();
  });

  it("has nowhere to climb from a project root", () => {
    expect(climbTarget("Inbox", "Inbox")).toBeNull();
  });
});

describe("childDirs", () => {
  it("collects every folder a level found", () => {
    const level = [listing([row("p/a", true), row("p/b.pdf", false), row("p/c", true)])];

    expect(childDirs(level, 10)).toEqual(["p/a", "p/c"]);
  });

  /// The depth cap bounds the rounds, not the width. One Option-click over a
  /// folder holding a JS checkout is thousands of package directories on a
  /// single level, each one an invoke, a row, and a line in the persisted
  /// expansion that every launch then re-reads.
  it("stops at the budget however wide the level is", () => {
    const wide = listing(Array.from({ length: 5000 }, (_, n) => row(`p/pkg${String(n)}`, true)));

    expect(childDirs([wide], 200)).toHaveLength(200);
    expect(childDirs([wide], 0)).toEqual([]);
  });

  it("skips a folder that failed to list", () => {
    expect(childDirs([null, listing([row("p/a", true)])], 10)).toEqual(["p/a"]);
  });
});
