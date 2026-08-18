import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ServiceRequestPayload, ServiceResponsePayload } from "./transport";

/// Typed so the assertions below read the payload through the same contract
/// the host does. An untyped `vi.fn()` makes every `.path` / `.body` an `any`,
/// which is exactly the check worth keeping on the module that builds them.
const invoke = vi.hoisted(() =>
  vi.fn<
    (cmd: string, args: { request: ServiceRequestPayload }) => Promise<ServiceResponsePayload>
  >(),
);
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { hostFetch } = await import("./transport");

/// The seam between openapi-fetch and the host. It is the only place the
/// contract's URLs are taken apart, and a mistake here is invisible in types:
/// dropping the query string or the body still compiles, and only shows up as
/// a conversion that silently ignores its options.
describe("hostFetch", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue({ status: 200, headers: {}, body: "{}" });
  });

  const payload = (): ServiceRequestPayload => invoke.mock.calls[0][1].request;

  it("forwards path and query, dropping the nominal base URL", async () => {
    await hostFetch("http://127.0.0.1:8080/api/v1/conversions/abc?format=markdown");
    expect(invoke).toHaveBeenCalledWith("service_request", expect.anything());
    expect(payload().path).toBe("/api/v1/conversions/abc?format=markdown");
  });

  it("defaults to GET and carries method, headers, and a serialized body", async () => {
    await hostFetch("http://127.0.0.1:8080/api/v1/conversions");
    expect(payload().method).toBe("GET");
    expect(payload().body).toBeUndefined();

    invoke.mockClear();
    await hostFetch("http://127.0.0.1:8080/api/v1/conversions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: '{"source":"/tmp/a.pdf"}',
    });
    expect(payload().method).toBe("POST");
    expect(payload().headers["content-type"]).toBe("application/json");
    expect(payload().body).toBe('{"source":"/tmp/a.pdf"}');
  });

  it("accepts a URL as well as a string", async () => {
    await hostFetch(new URL("http://127.0.0.1:8080/api/v1/health"));
    expect(payload().path).toBe("/api/v1/health");
  });

  /// The signature is `typeof fetch`, so a Request must be honoured whole.
  /// Reading only `init` would turn this into a GET with no headers and no
  /// body — and it would still type-check.
  it("takes method, headers, and body from a Request, not just from init", async () => {
    await hostFetch(
      new Request("http://127.0.0.1:8080/api/v1/conversions?wait=1", {
        method: "POST",
        headers: { "content-type": "application/json", "x-trace": "abc" },
        body: '{"source":"/tmp/a.pdf"}',
      }),
    );
    expect(payload().path).toBe("/api/v1/conversions?wait=1");
    expect(payload().method).toBe("POST");
    expect(payload().headers["content-type"]).toBe("application/json");
    expect(payload().headers["x-trace"]).toBe("abc");
    expect(payload().body).toBe('{"source":"/tmp/a.pdf"}');
  });

  it("lets init override the Request, the way fetch merges them", async () => {
    await hostFetch(
      new Request("http://127.0.0.1:8080/api/v1/conversions", {
        method: "POST",
        headers: { "x-trace": "from-request", "x-keep": "yes" },
        body: '{"from":"request"}',
      }),
      { method: "PUT", headers: { "x-trace": "from-init" }, body: '{"from":"init"}' },
    );
    expect(payload().method).toBe("PUT");
    expect(payload().headers["x-trace"]).toBe("from-init");
    // Headers the init did not mention survive rather than being wiped.
    expect(payload().headers["x-keep"]).toBe("yes");
    expect(payload().body).toBe('{"from":"init"}');
  });

  it("rejects a non-text body rather than sending an empty one", async () => {
    await expect(
      hostFetch("http://127.0.0.1:8080/api/v1/conversions", {
        method: "POST",
        body: new Uint8Array([1, 2, 3]),
      }),
    ).rejects.toThrow(/pre-serialized text body/);
    expect(invoke).not.toHaveBeenCalled();
  });

  /// A 4xx must come back as an ordinary Response so openapi-fetch types it as
  /// `error` and hands the caller the contract's ErrorEnvelope. Throwing here
  /// would turn every expected failure into an unhandled rejection.
  it("returns non-2xx as a Response instead of throwing", async () => {
    invoke.mockResolvedValue({
      status: 404,
      headers: { "content-type": "application/json" },
      body: '{"error":{"code":"not_found"}}',
    });
    const res = await hostFetch("http://127.0.0.1:8080/api/v1/conversions/missing");
    expect(res.status).toBe(404);
    expect(await res.json()).toEqual({ error: { code: "not_found" } });
  });
});
