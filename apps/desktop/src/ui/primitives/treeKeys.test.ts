import { describe, expect, it } from "vitest";
import {
  isTypeAheadKey,
  parentIndex,
  treeAction,
  typeAheadTarget,
  type TreeKeyPress,
  type TreeRowInfo,
} from "./treeKeys";

/// Inbox            open branch
///   welcome.md
///   Deals          open branch
///     msa.pdf
///     sow.docx
///   prices.csv
/// Archive          closed branch
const ROWS: TreeRowInfo[] = [
  { path: "Inbox", depth: 0, open: true, name: "Inbox" },
  { path: "Inbox/welcome.md", depth: 1, name: "welcome.md" },
  { path: "Inbox/Deals", depth: 1, open: true, name: "Deals" },
  { path: "Inbox/Deals/msa.pdf", depth: 2, name: "msa.pdf" },
  { path: "Inbox/Deals/sow.docx", depth: 2, name: "sow.docx" },
  { path: "Inbox/prices.csv", depth: 1, name: "prices.csv" },
  { path: "Archive", depth: 0, open: false, name: "Archive" },
];

const press = (key: string, mods: Partial<TreeKeyPress> = {}): TreeKeyPress => ({
  key,
  altKey: false,
  metaKey: false,
  ctrlKey: false,
  ...mods,
});

describe("treeAction", () => {
  it("moves the selection with the arrows and opens nothing", () => {
    expect(treeAction(ROWS, "Inbox/welcome.md", press("ArrowDown"))).toEqual({
      kind: "select",
      path: "Inbox/Deals",
    });
    expect(treeAction(ROWS, "Inbox/welcome.md", press("ArrowUp"))).toEqual({
      kind: "select",
      path: "Inbox",
    });
  });

  it("stops at both ends rather than cycling", () => {
    expect(treeAction(ROWS, "Inbox", press("ArrowUp"))).toBeNull();
    expect(treeAction(ROWS, "Archive", press("ArrowDown"))).toBeNull();
  });

  it("lands on the first row when nothing is selected yet", () => {
    expect(treeAction(ROWS, null, press("ArrowUp"))).toEqual({ kind: "select", path: "Inbox" });
    expect(treeAction(ROWS, null, press("ArrowDown"))).toEqual({ kind: "select", path: "Inbox" });
  });

  it("opens a closed branch with Right and descends into an open one", () => {
    expect(treeAction(ROWS, "Archive", press("ArrowRight"))).toEqual({
      kind: "toggle",
      path: "Archive",
      open: true,
      deep: false,
    });
    expect(treeAction(ROWS, "Inbox/Deals", press("ArrowRight"))).toEqual({
      kind: "select",
      path: "Inbox/Deals/msa.pdf",
    });
  });

  it("leaves Right alone on a leaf", () => {
    expect(treeAction(ROWS, "Inbox/welcome.md", press("ArrowRight"))).toBeNull();
  });

  it("closes an open branch with Left and climbs from a leaf", () => {
    expect(treeAction(ROWS, "Inbox/Deals", press("ArrowLeft"))).toEqual({
      kind: "toggle",
      path: "Inbox/Deals",
      open: false,
      deep: false,
    });
    expect(treeAction(ROWS, "Inbox/Deals/sow.docx", press("ArrowLeft"))).toEqual({
      kind: "select",
      path: "Inbox/Deals",
    });
    // A closed branch climbs too, or the key goes dead on the row it just
    // closed. Archive is top level, so there is nowhere above it to go.
    expect(treeAction(ROWS, "Archive", press("ArrowLeft"))).toBeNull();
  });

  it("takes the whole subtree with Option", () => {
    expect(treeAction(ROWS, "Inbox", press("ArrowRight", { altKey: true }))).toEqual({
      kind: "toggle",
      path: "Inbox",
      open: true,
      deep: true,
    });
    expect(treeAction(ROWS, "Inbox", press("ArrowLeft", { altKey: true }))).toEqual({
      kind: "toggle",
      path: "Inbox",
      open: false,
      deep: true,
    });
  });

  it("jumps to the ends with Home and End", () => {
    expect(treeAction(ROWS, "Inbox/Deals", press("Home"))).toEqual({
      kind: "select",
      path: "Inbox",
    });
    expect(treeAction(ROWS, "Inbox/Deals", press("End"))).toEqual({
      kind: "select",
      path: "Archive",
    });
    expect(treeAction(ROWS, "Inbox", press("Home"))).toBeNull();
  });

  it("separates activation from movement", () => {
    expect(treeAction(ROWS, "Inbox/prices.csv", press("Enter"))).toEqual({
      kind: "activate",
      path: "Inbox/prices.csv",
    });
    expect(treeAction(ROWS, "Inbox/prices.csv", press("ArrowDown", { metaKey: true }))).toEqual({
      kind: "activate",
      path: "Inbox/prices.csv",
    });
    expect(treeAction(ROWS, "Inbox/Deals/msa.pdf", press("ArrowUp", { metaKey: true }))).toEqual({
      kind: "select",
      path: "Inbox/Deals",
    });
    expect(treeAction(ROWS, "Inbox/prices.csv", press(" "))).toEqual({
      kind: "inspect",
      path: "Inbox/prices.csv",
    });
  });

  it("hands back keys that are not its own", () => {
    expect(treeAction(ROWS, "Inbox", press("Tab"))).toBeNull();
    expect(treeAction(ROWS, "Inbox", press("Escape"))).toBeNull();
    expect(treeAction([], null, press("ArrowDown"))).toBeNull();
  });
});

describe("parentIndex", () => {
  it("finds the enclosing row, not the previous one", () => {
    expect(parentIndex(ROWS, 4)).toBe(2);
    expect(parentIndex(ROWS, 5)).toBe(0);
    expect(parentIndex(ROWS, 0)).toBe(-1);
  });
});

describe("typeAheadTarget", () => {
  it("matches a prefix, case-insensitively", () => {
    expect(typeAheadTarget(ROWS, null, "we")).toBe("Inbox/welcome.md");
    expect(typeAheadTarget(ROWS, null, "SO")).toBe("Inbox/Deals/sow.docx");
  });

  it("walks the matches on a repeated letter and wraps", () => {
    // Searches past the current row, then round the end of the list. With one
    // match in the tree it comes back to the row it started on.
    expect(typeAheadTarget(ROWS, "Inbox/prices.csv", "m")).toBe("Inbox/Deals/msa.pdf");
    expect(typeAheadTarget(ROWS, "Inbox", "i")).toBe("Inbox");
  });

  it("keeps the row it found while the buffer grows", () => {
    expect(typeAheadTarget(ROWS, "Inbox/Deals/msa.pdf", "ms")).toBe("Inbox/Deals/msa.pdf");
  });

  it("reports nothing rather than moving on a miss", () => {
    expect(typeAheadTarget(ROWS, "Inbox", "zz")).toBeNull();
    expect(typeAheadTarget(ROWS, "Inbox", "")).toBeNull();
  });
});

describe("isTypeAheadKey", () => {
  it("takes bare characters and leaves commands and named keys alone", () => {
    expect(isTypeAheadKey(press("w"))).toBe(true);
    expect(isTypeAheadKey(press(" "))).toBe(false);
    expect(isTypeAheadKey(press("Tab"))).toBe(false);
    expect(isTypeAheadKey(press("w", { metaKey: true }))).toBe(false);
  });
});
