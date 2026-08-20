/// The conversion-service API layer. `schema.ts` is generated from
/// `contract/http/openapi.yaml` by `pnpm generate:api` and is imported only
/// inside this directory; everything the app needs comes through this barrel.

export { conversionClient } from "./client";
export type { ServiceRequestPayload, ServiceResponsePayload } from "./transport";
export type { components, operations, paths } from "./schema";
