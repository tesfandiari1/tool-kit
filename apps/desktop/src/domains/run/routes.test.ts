import { describe, expect, it } from "vitest";
import type { ScannedConversionFile, SecretStatus } from "@/app/types";
import {
  missingConversionCredentials,
  planConversionRoutes,
  type ConversionCapabilities,
} from "./routes";

const secrets = (saved: Partial<SecretStatus> = {}): SecretStatus => ({
  datalab: false,
  revai: false,
  backend: false,
  ...saved,
});

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
  it("uses the live inputFormats list instead of a built-in backend format list", () => {
    const pdf = file("/drop/report.pdf", "application/pdf");
    const docx = file(
      "/drop/report.docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );

    const first = planConversionRoutes({
      files: [pdf, docx],
      route: "backend",
      profile: "standard",
      capabilities: ready(["application/pdf"]),
      skipAlreadyDone: true,
    });
    expect(first.backend).toEqual([pdf]);
    expect(first.direct).toEqual([docx]);

    const second = planConversionRoutes({
      files: [pdf, docx],
      route: "backend",
      profile: "standard",
      capabilities: ready([docx.mediaType]),
      skipAlreadyDone: true,
    });
    expect(second.backend).toEqual([docx]);
    expect(second.direct).toEqual([pdf]);
  });

  it("plans mixed routes and requires only the credentials those files use", () => {
    const pdf = file("/drop/report.pdf", "application/pdf");
    const image = file("/drop/scan.png", "image/png");
    const plan = planConversionRoutes({
      files: [pdf, image],
      route: "backend",
      profile: "standard",
      capabilities: ready([pdf.mediaType]),
      skipAlreadyDone: true,
    });

    expect(plan.backend).toEqual([pdf]);
    expect(plan.direct).toEqual([image]);
    expect(plan.blocked).toEqual([]);
    expect(missingConversionCredentials("convert", plan, secrets())).toEqual(["datalab", "backend"]);
    expect(missingConversionCredentials("convert", plan, secrets({ datalab: true }))).toEqual(["backend"]);
    expect(missingConversionCredentials("convert", plan, secrets({ datalab: true, backend: true }))).toEqual([]);
  });

  it("keeps HTML direct but routes images on what the service advertises", () => {
    const image = file("/drop/photo.JPEG", "image/jpeg");
    const page = file("C:\\drop\\page.HTM", "text/html");
    const plan = planConversionRoutes({
      files: [image, page],
      route: "backend",
      profile: "standard",
      capabilities: ready([image.mediaType, page.mediaType]),
      skipAlreadyDone: true,
    });

    // The Vision engine is live, so the image is local work. HTML has no local
    // engine at all and stays direct however the service answers.
    expect(plan.direct).toEqual([page]);
    expect(plan.backend).toEqual([image]);
    expect(plan.needsDatalabKey).toBe(true);
    expect(plan.needsBackendToken).toBe(true);

    // No engine — a Linux deployment, or a Mac below macOS 26 — and the same
    // image goes back to Datalab.
    const noVision = planConversionRoutes({
      files: [image, page],
      route: "backend",
      profile: "standard",
      capabilities: ready([page.mediaType]),
      skipAlreadyDone: true,
    });
    expect(noVision.direct).toEqual([image, page]);
    expect(noVision.backend).toEqual([]);
  });

  it("lets permanent-direct files proceed when the backend is unavailable but blocks candidates", () => {
    const page = file("/drop/page.html", "text/html");
    const image = file("/drop/scan.tif", "image/tiff");
    const document = file("/drop/report.pdf", "application/pdf");
    const plan = planConversionRoutes({
      files: [page, image, document],
      route: "backend",
      profile: "standard",
      capabilities: { state: "unavailable" },
      skipAlreadyDone: true,
    });

    expect(plan.direct).toEqual([page]);
    expect(plan.backend).toEqual([]);
    // An image is a backend candidate now, so an unreachable service blocks it
    // rather than handing it to Datalab behind the user's back.
    expect(plan.blocked).toEqual([
      { file: image, reason: "backend_unavailable" },
      { file: document, reason: "backend_unavailable" },
    ]);
  });

  it("never falls back to a remote provider in local-only mode", () => {
    const image = file("/drop/scan.webp", "image/webp");
    const unsupported = file("/drop/report.docx", "application/x-not-supported-today");
    const supported = file("/drop/report.pdf", "application/pdf");
    const plan = planConversionRoutes({
      files: [image, unsupported, supported],
      route: "backend",
      profile: "local_only",
      capabilities: ready([supported.mediaType]),
      skipAlreadyDone: true,
    });

    expect(plan.direct).toEqual([]);
    expect(plan.backend).toEqual([supported]);
    expect(plan.blocked).toEqual([
      { file: image, reason: "local_only_requires_remote" },
      { file: unsupported, reason: "local_only_requires_remote" },
    ]);
  });

  it("keeps the direct switch on the current all-Datalab behavior without capabilities", () => {
    const pdf = file("/drop/report.pdf", "application/pdf");
    const plan = planConversionRoutes({
      files: [pdf],
      route: "direct",
      profile: "local_only",
      capabilities: { state: "unavailable" },
      skipAlreadyDone: true,
    });

    expect(plan.direct).toEqual([pdf]);
    expect(plan.backend).toEqual([]);
    expect(plan.blocked).toEqual([]);
  });

  it("omits reusable files only while skip-already-done is enabled", () => {
    const reused = file("/drop/report.pdf", "application/pdf", "reusable");
    const enabled = planConversionRoutes({
      files: [reused],
      route: "backend",
      profile: "standard",
      capabilities: ready([reused.mediaType]),
      skipAlreadyDone: true,
    });
    const disabled = planConversionRoutes({
      files: [reused],
      route: "backend",
      profile: "standard",
      capabilities: ready([reused.mediaType]),
      skipAlreadyDone: false,
    });

    expect(enabled.backend).toEqual([]);
    expect(disabled.backend).toEqual([reused]);
  });

  /// The flagship path: a local transcription spends no Rev.ai credits, and an
  /// unadvertised container is a blocked preflight, not a failed run.
  it("asks a backend transcription for the backend token alone", () => {
    const m4a = file("/drop/interview.m4a", "audio/mp4");
    const amr = file("/drop/voicemail.amr", "audio/amr");
    const local = planConversionRoutes({
      files: [m4a],
      route: "backend",
      profile: "standard",
      capabilities: ready([m4a.mediaType]),
      skipAlreadyDone: true,
    });

    expect(local.backend).toEqual([m4a]);
    expect(missingConversionCredentials("transcribe", local, secrets({ backend: true }))).toEqual(
      [],
    );

    const mixed = planConversionRoutes({
      files: [m4a, amr],
      route: "backend",
      profile: "local_only",
      capabilities: ready([m4a.mediaType]),
      skipAlreadyDone: true,
    });
    expect(mixed.blocked).toEqual([{ file: amr, reason: "local_only_requires_remote" }]);
    expect(missingConversionCredentials("transcribe", mixed, secrets())).toEqual(["backend"]);

    const direct = planConversionRoutes({
      files: [m4a],
      route: "direct",
      profile: "standard",
      capabilities: ready([m4a.mediaType]),
      skipAlreadyDone: true,
    });
    expect(missingConversionCredentials("transcribe", direct, secrets())).toEqual(["revai"]);
  });

  it("blocks supported files while the service is not accepting jobs", () => {
    const pdf = file("/drop/report.pdf", "application/pdf");
    const plan = planConversionRoutes({
      files: [pdf],
      route: "backend",
      profile: "standard",
      capabilities: ready([pdf.mediaType], false),
      skipAlreadyDone: true,
    });

    expect(plan.blocked).toEqual([{ file: pdf, reason: "backend_not_accepting" }]);
  });
});
