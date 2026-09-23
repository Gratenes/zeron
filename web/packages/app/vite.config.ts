import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

const engine = process.env.ZERON_DEV_ENGINE ?? "http://127.0.0.1:27699";

export default defineConfig({
  plugins: [react()],
  build: {
    rollupOptions: {
      input: {
        index: resolve(__dirname, "index.html"),
        pair: resolve(__dirname, "pair.html"),
      },
    },
  },
  server: {
    /*
     * The engine serves its RPC WebSocket at path `/` — the very path Vite's
     * own HMR socket uses — so the catch-all proxy below cannot tell them
     * apart by URL and would hand HMR to the engine, which rejects it. A
     * rejected HMR socket makes Vite's client reload the page, and it retries
     * forever: a reload loop. Give HMR its own port so it never touches the
     * proxy at all.
     */
    hmr: { port: 24678 },
    proxy: {
      // The pre-auth sign-in routes the browser calls over plain HTTP
      // before any credential or WebSocket exists. Explicit entries
      // because the catch-all below serves non-WebSocket requests from
      // Vite itself (the engine serves the app in production, but here
      // Vite owns the HTML).
      "/auth/config": { target: engine },
      "/auth/exchange": { target: engine },
      "/": {
        target: engine,
        ws: true,
        bypass: (req) => {
          // Belt and braces: even on one port, never proxy Vite's own socket.
          const upgrade = req.headers.upgrade;
          const protocol = req.headers["sec-websocket-protocol"];
          if (upgrade === "websocket" && !String(protocol ?? "").includes("vite-hmr")) {
            return undefined;
          }
          return req.url;
        },
      },
    },
  },
});
