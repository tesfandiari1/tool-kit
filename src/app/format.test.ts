import { describe, expect, it } from "vitest";
import { basename, fmtElapsed, fmtWhen } from "./format";

describe("basename", () => {
  it("returns the last path component on POSIX and Windows separators", () => {
    expect(basename("/tmp/out/notes.md")).toBe("notes.md");
    expect(basename("C:\\Users\\a\\notes.md")).toBe("notes.md");
    expect(basename("notes.md")).toBe("notes.md");
    expect(basename("/")).toBe("/");
  });
});

describe("fmtElapsed", () => {
  it("formats a zero-padded mm:ss clock from unix seconds", () => {
    expect(fmtElapsed(1_700_000_000_000, 1_700_000_000)).toBe("0:00");
    expect(fmtElapsed(1_700_000_065_000, 1_700_000_000)).toBe("1:05");
    expect(fmtElapsed(1_699_000_000_000, 1_700_000_000)).toBe("0:00");
  });
});

describe("fmtWhen", () => {
  const now = 1_700_000_000_000;

  it("uses relative labels while they are the useful answer", () => {
    expect(fmtWhen(1_700_000_000 - 30, now)).toBe("just now");
    expect(fmtWhen(1_700_000_000 - 120, now)).toBe("2m ago");
    expect(fmtWhen(1_700_000_000 - 7200, now)).toBe("2h ago");
    expect(fmtWhen(1_700_000_000 - 90_000, now)).toBe("yesterday");
  });
});
