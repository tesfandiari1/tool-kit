import { describe, expect, it } from "vitest";
import { isDirty, saveNote, saveTone, type SaveState } from "./model";

describe("isDirty", () => {
  it("counts a refused save as unsaved work", () => {
    expect(isDirty("error")).toBe(true);
  });

  it("counts an edit as unsaved work", () => {
    expect(isDirty("edited")).toBe(true);
  });

  // `saving` reading as clean is what stops ⌘S from racing the autosave
  // timer into a second write of the same text.
  it("leaves a write in flight, a finished write and an untouched file alone", () => {
    const clean: SaveState[] = ["clean", "saving", "saved"];
    expect(clean.map(isDirty)).toEqual([false, false, false]);
  });

  // The tab's dirty mark and the close confirm must agree, or a document whose
  // save was refused looks settled in the strip and then asks on the way out.
  it("marks every state the close confirm asks about", () => {
    const states: SaveState[] = ["clean", "edited", "saving", "saved", "error"];
    const asks = states.filter(isDirty);
    expect(asks).toEqual(["edited", "error"]);
  });
});

describe("saveTone / saveNote", () => {
  it("says nothing about a document nobody has touched", () => {
    expect(saveNote("clean")).toBeNull();
    expect(saveTone("clean")).toBe("idle");
  });

  it("gives a refused write the fault tone, not a quiet one", () => {
    expect(saveTone("error")).toBe("fault");
    expect(saveNote("error")).toBe("Could not save");
  });
});
