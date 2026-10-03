import { describe, expect, it } from "vitest";
import { activateOrInsert, removeDoc, renameDoc } from "./documents";
import type { OpenDoc } from "@/domains/thread/model";

function doc(id: string, save: OpenDoc["save"] = "clean"): OpenDoc {
  return { id, text: "", save, mtimeMs: 0 };
}

const a = doc("/out/a.md");
const b = doc("/out/b.md");
const c = doc("/out/c.md");

describe("activateOrInsert", () => {
  it("opens a document and makes it active", () => {
    expect(activateOrInsert([a], b)).toEqual({ docs: [a, b], activeId: b.id });
  });

  it("activates an already open document instead of opening it twice", () => {
    expect(activateOrInsert([a, b], doc(a.id))).toEqual({ docs: [a, b], activeId: a.id });
  });

  it("keeps the open copy, edits and all, when the same path is opened again", () => {
    const edited = doc(a.id, "edited");
    expect(activateOrInsert([edited], doc(a.id)).docs[0].save).toBe("edited");
  });
});

describe("removeDoc", () => {
  it("empties the pane when the last document closes", () => {
    expect(removeDoc([a], a.id, a.id)).toEqual({ docs: [], activeId: null });
  });

  it("activates the document that took the closed one's place", () => {
    expect(removeDoc([a, b, c], b.id, b.id)).toEqual({ docs: [a, c], activeId: c.id });
  });

  it("falls back to the previous document when the last tab closes", () => {
    expect(removeDoc([a, b], b.id, b.id)).toEqual({ docs: [a], activeId: a.id });
  });

  it("leaves the active document showing when another one closes", () => {
    expect(removeDoc([a, b, c], c.id, a.id)).toEqual({ docs: [b, c], activeId: c.id });
  });

  it("changes nothing when the document is not open", () => {
    expect(removeDoc([a], a.id, b.id)).toEqual({ docs: [a], activeId: a.id });
  });
});

describe("renameDoc", () => {
  it("carries the open tab to the file's new path", () => {
    const moved = renameDoc([a, b], a.id, a.id, "/taxes/a.md");
    expect(moved.docs[0].id).toBe("/taxes/a.md");
    expect(moved.activeId).toBe("/taxes/a.md");
  });

  it("keeps an unsaved edit, which is the whole point of following the file", () => {
    const edited = doc(a.id, "edited");
    expect(renameDoc([edited], a.id, a.id, "/taxes/a.md").docs[0].save).toBe("edited");
  });

  it("leaves the showing document alone when another one moves", () => {
    const moved = renameDoc([a, b], b.id, a.id, "/taxes/a.md");
    expect(moved.activeId).toBe(b.id);
  });

  it("changes nothing when the moved file was never open", () => {
    expect(renameDoc([a], a.id, c.id, "/taxes/c.md")).toEqual({ docs: [a], activeId: a.id });
  });
});
