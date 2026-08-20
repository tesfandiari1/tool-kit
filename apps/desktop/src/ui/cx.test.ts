import { describe, expect, it } from "vitest";
import { cx } from "./cx";

describe("cx", () => {
  it("joins non-empty strings and drops the usual conditional leftovers", () => {
    const classIf = (on: boolean) => cx("a", on && "b", "c");
    expect(classIf(false)).toBe("a c");
    expect(classIf(true)).toBe("a b c");
    expect(cx("a", 0, null, undefined, "", "b")).toBe("a b");
    expect(cx()).toBe("");
  });
});
