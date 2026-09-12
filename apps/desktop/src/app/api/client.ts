import createClient from "openapi-fetch";
import type { paths } from "./schema";
import { hostFetch } from "./transport";

/// An openapi-fetch client over the schema generated from the contract, so
/// results arrive as `{ data, error }` with the contract's ErrorEnvelope.
/// `hostFetch` runs the HTTP in the host, and the base URL here is nominal.
export const conversionClient = createClient<paths>({
  baseUrl: "http://127.0.0.1:8080",
  fetch: hostFetch,
});
