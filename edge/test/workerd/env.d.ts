/// <reference types="@cloudflare/vitest-plugin" />

declare module "cloudflare:test" {
  interface ProvidedEnv {
    TEST_LOG: DurableObjectNamespace;
    PREVIEW_ROOMS: DurableObjectNamespace;

    BROWSER_SESSIONS: DurableObjectNamespace;

    DEVICE_ROOMS: DurableObjectNamespace;
  }
}
