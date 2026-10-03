import { describe, expect, it } from "vitest";
import type { ScannedConversionFile } from "@/app/types";
import { planConversionRoutes, type ConversionCapabilities } from "./routes";

const file = (
  sourcePath: string,
  mediaType: string,
  reuse: ScannedConversionFile["reuse"] = "pending",
): ScannedConversionFile => ({ sourcePath, mediaType, reuse });

const ready = (inputFormats: readonly string[], acceptingJobs = true): ConversionCapabilities => ({
  state: "ready",
  acceptingJobs,
  inputFormats,
});

describe("planConversionRoutes", () => {
  /// Nothing leaves the Mac, so a file the service does not advertise is a
  /// blocked preflight, never a remote upload.
  it("sends what the live service advertises and blocks the rest", () => {
    const pdf = file("/drop/report.pdf", "application/pdf");
    const image = file("/drop/photo.JPEG", "image/jpeg");
    const plan = planConversionRoutes({
      files: [pdf, image],
      capabilities: ready([pdf.mediaType]),
      skipAlreadyDone: true,
    });

    expect(plan.backend).toEqual([pdf]);
    expect(plan.blocked).toEqual([{ file: image, reason: "not_supported" }]);
  });

  it("blocks every file while the service is unavailable or still answering", () => {
    const pdf = file("/drop/report.pdf", "application/pdf");
    const call = file("/drop/call.wav", "audio/wav");
    const down = planConversionRoutes({
      files: [pdf, call],
      capabilities: { state: "unavailable" },
      skipAlreadyDone: true,
    });
    expect(down.backend).toEqual([]);
    expect(down.blocked).toEqual([
      { file: pdf, reason: "backend_unavailable" },
      { file: call, reason: "backend_unavailable" },
    ]);

    const pending = planConversionRoutes({
      files: [pdf],
      capabilities: { state: "loading" },
      skipAlreadyDone: true,
    });
    expect(pending.blocked).toEqual([{ file: pdf, reason: "capabilities_pending" }]);
  });

  it("omits reusable files only while skip-already-done is enabled", () => {
    const reused = file("/drop/report.pdf", "application/pdf", "reusable");
    const enabled = planConversionRoutes({
      files: [reused],
      capabilities: ready([reused.mediaType]),
      skipAlreadyDone: true,
    });
    const disabled = planConversionRoutes({
      files: [reused],
      capabilities: ready([reused.mediaType]),
      skipAlreadyDone: false,
    });

    expect(enabled.backend).toEqual([]);
    expect(disabled.backend).toEqual([reused]);
  });

  it("blocks supported files while the service is not accepting jobs", () => {
    const pdf = file("/drop/report.pdf", "application/pdf");
    const plan = planConversionRoutes({
      files: [pdf],
      capabilities: ready([pdf.mediaType], false),
      skipAlreadyDone: true,
    });

    expect(plan.blocked).toEqual([{ file: pdf, reason: "backend_not_accepting" }]);
  });
});
