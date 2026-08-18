import { commands } from "../commands";

/// What the host's `service_request` command receives: everything it needs to
/// replay the request against the conversion service. The host resolves the
/// real base URL from settings, attaches the bearer token from the Keychain,
/// and for `createConversion` streams the multipart source from disk. The
/// webview never sees the token and never reads file bytes.
export interface ServiceRequestPayload {
  method: string;
  /// Path plus query, e.g. `/api/v1/conversions/9f…?…`. The host joins it
  /// onto the configured backend URL.
  path: string;
  headers: Record<string, string>;
  /// The serialized JSON body, when the operation has one. For
  /// `createConversion` this is the `ConversionSubmission` fields as JSON,
  /// with `source` holding a desktop path the host streams from.
  body?: string;
}

/// What `service_request` answers with. `body` is the raw response text;
/// openapi-fetch parses it by content type (JSON envelopes, text/markdown).
export interface ServiceResponsePayload {
  status: number;
  headers: Record<string, string>;
  body: string;
}

/// A `fetch` that runs in the host instead of the webview. Plugged into
/// openapi-fetch's documented `fetch` seam, so the client surface stays 100%
/// standard while the network, the token, and the file streaming stay in Rust
/// (the M6 gate). Non-2xx answers come back as ordinary Responses carrying
/// the service's ErrorEnvelope, which openapi-fetch types and returns as
/// `error` rather than throwing.
///
/// One deliberate limit, fine for the six operations in the contract:
/// `AbortSignal` is not bridged yet.
///
/// The signature is `typeof fetch`, so a `Request` has to be honoured as a
/// whole. Reading only `init` would silently turn
/// `hostFetch(new Request(url, { method: "POST", body }))` into a GET with no
/// headers and no body — a request that type-checks, reaches the host, and
/// does the wrong thing. openapi-fetch passes `(url, init)` today, so nothing
/// exercises that path yet, which is exactly why it has to be closed now
/// rather than discovered at M6.
export const hostFetch: typeof fetch = async (input, init) => {
  const req = input instanceof Request ? input : null;
  const href = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
  const url = new URL(href);

  // `init` wins over the Request, matching how the real `fetch` merges them.
  const method = init?.method ?? req?.method ?? "GET";
  const headers = new Headers(req?.headers);
  if (init?.headers) for (const [k, v] of new Headers(init.headers)) headers.set(k, v);

  let body: string | undefined;
  if (init?.body != null) {
    if (typeof init.body !== "string") {
      throw new Error("hostFetch: expected a pre-serialized text body");
    }
    body = init.body;
  } else if (req && req.body !== null) {
    // A Request's body is a stream and can only be drained once. This clone
    // is what lets the caller keep holding a usable Request.
    body = await req.clone().text();
  }

  const res = await commands.serviceRequest({
    method,
    path: url.pathname + url.search,
    headers: Object.fromEntries(headers),
    body,
  });
  return new Response(res.body, { status: res.status, headers: res.headers });
};
