import createClient from "openapi-fetch";
import type { paths } from "./schema";
import { hostFetch } from "./transport";

/// The conversion-service client. A standard openapi-fetch client over the
/// schema generated from `contract/http/openapi.yaml`: every path, method,
/// body, response, and error type comes from the contract, and results arrive
/// as the library's `{ data, error }` unions, with `error` carrying the
/// contract's ErrorEnvelope.
///
/// HTTP itself runs in the host. `hostFetch` forwards each request through
/// the `service_request` Tauri command, so the bearer token never enters the
/// webview. The base URL is nominal; the host substitutes the configured one.
///
///     const { data, error } = await conversionClient.GET("/api/v1/conversions/{id}", {
///       params: { path: { id } },
///     });
///     // data is the JobEnvelope; data.data is the ConversionJob.
export const conversionClient = createClient<paths>({
  baseUrl: "http://127.0.0.1:8080",
  fetch: hostFetch,
});
