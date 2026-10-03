import { describe, expect, it } from "vitest";
import { jobDetailItems, jobDetailText } from "./details";

describe("jobDetailItems", () => {
  it("returns no details for a job without backend metadata", () => {
    expect(jobDetailItems({ warnings: [], failure: null, error: null })).toEqual([]);
  });

  it("preserves future warning values without enumerating them", () => {
    expect(
      jobDetailItems({
        warnings: ["future_warning", "another_future_warning"],
        failure: null,
        error: null,
      }),
    ).toEqual([
      { kind: "warning", label: "Warning", value: "future_warning" },
      { kind: "warning", label: "Warning", value: "another_future_warning" },
    ]);
  });

  it("surfaces both the failure code and its message", () => {
    expect(
      jobDetailItems({
        warnings: [],
        failure: { code: "future_failure", message: "The service explained what happened." },
        error: null,
      }),
    ).toEqual([
      {
        kind: "failure",
        label: "Failure",
        value: "future_failure: The service explained what happened.",
      },
    ]);
  });

  // `fail()` writes the backend failure into `error` as `code: message`, and
  // the row prints `error` on its own line.
  it("drops a failure the row's error already says", () => {
    expect(
      jobDetailItems({
        warnings: [],
        failure: { code: "bad_document", message: "Document cannot be converted" },
        error: "bad_document: Document cannot be converted",
      }),
    ).toEqual([]);
  });

  it("formats every returned detail into the visible queue text", () => {
    const items = jobDetailItems({
      warnings: ["future_warning"],
      failure: { code: "future_failure", message: "Future-safe message" },
      error: null,
    });

    expect(jobDetailText(items)).toBe(
      "Warning: future_warning · Failure: future_failure: Future-safe message",
    );
  });
});
