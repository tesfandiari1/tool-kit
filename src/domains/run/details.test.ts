import { describe, expect, it } from "vitest";
import { jobDetailItems, jobDetailText } from "./details";

describe("jobDetailItems", () => {
  it("returns no details for a direct job without backend metadata", () => {
    expect(
      jobDetailItems({ route: null, reasonCodes: [], warnings: [], failure: null }),
    ).toEqual([]);
  });

  it("preserves future route, reason, and warning values without enumerating them", () => {
    expect(
      jobDetailItems({
        route: "future_hybrid_route",
        reasonCodes: ["future_reason", "another_future_reason"],
        warnings: ["future_warning"],
        failure: null,
      }),
    ).toEqual([
      { kind: "route", label: "Route", value: "future_hybrid_route" },
      { kind: "reason", label: "Reason", value: "future_reason" },
      { kind: "reason", label: "Reason", value: "another_future_reason" },
      { kind: "warning", label: "Warning", value: "future_warning" },
    ]);
  });

  it("surfaces both the failure code and its message", () => {
    expect(
      jobDetailItems({
        route: null,
        reasonCodes: [],
        warnings: [],
        failure: { code: "future_failure", message: "The service explained what happened." },
      }),
    ).toEqual([
      {
        kind: "failure",
        label: "Failure",
        value: "future_failure: The service explained what happened.",
      },
    ]);
  });

  it("formats every returned detail into the visible queue text", () => {
    const items = jobDetailItems({
      route: "future_route",
      reasonCodes: ["future_reason"],
      warnings: ["future_warning"],
      failure: { code: "future_failure", message: "Future-safe message" },
    });

    expect(jobDetailText(items)).toBe(
      "Route: future_route · Reason: future_reason · Warning: future_warning · Failure: future_failure: Future-safe message",
    );
  });
});
