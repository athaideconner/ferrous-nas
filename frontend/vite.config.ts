import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Dev server proxies the API to the Rust daemon so the SPA can call
// same-origin `/api/...` during development.
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
