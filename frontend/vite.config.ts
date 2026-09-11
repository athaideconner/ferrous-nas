import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Dev server proxies the API to the Rust daemon so the SPA can call
// same-origin `/api/...` during development.
//
// This defaults to plain http:// on purpose: the daemon serves HTTPS with a
// self-signed cert by default, but a browser only honours the session
// cookie's `Secure` flag when *its own* connection is HTTPS — proxying
// through an HTTPS backend to an HTTP Vite origin doesn't count, and the
// cookie would silently fail to persist. scripts/dev.sh therefore runs the
// backend with FERROUS_TLS=off for local development; production has no such
// issue; since the daemon serves the built SPA itself, everything shares one
// HTTPS origin. Point FERROUS_API elsewhere (http or https) to override.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: process.env.FERROUS_API ?? "http://localhost:4200",
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
  },
});
