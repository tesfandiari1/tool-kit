/// The conversion-service API layer. `schema.ts` is generated from
/// `contract/http/openapi.yaml` by `pnpm generate:api` and is imported only
/// inside this directory. Everything the app needs comes through this barrel.

export { conversionClient } from "./client";
