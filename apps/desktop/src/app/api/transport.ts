import { commands } from "../commands";

/// What `service_request` receives. The host attaches the Keychain token and
/// streams multipart sources from disk, and the webview sees neither.
export interface ServiceRequestPayload {
  method: string;
  /// Path plus query. The host joins it onto the configured base URL.
  path: string;
  headers: Record<string, string>;
  /// For `createConversion`, `source` holds a path the host streams.
  body?: string;
}

/// `body` is bounded raw text. The host refuses the Markdown artifact endpoint
/// here: the job lifecycle streams that one to disk.
export interface ServiceResponsePayload {
  status: number;
  headers: Record<string, string>;
  body: string;
}

/// A `fetch` that runs in the host, on openapi-fetch's documented seam. Non-2xx
/// comes back as a Response carrying the ErrorEnvelope. `AbortSignal` is not
/// bridged. A `Request` has to be honoured whole: reading only `init` turns a
/// POST into a GET that type-checks and reaches the host.
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
    // A Request's body is a stream and drains once, so the clone is what leaves
    // the caller holding a usable Request.
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
