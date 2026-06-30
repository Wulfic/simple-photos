import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";
import { readFileSync } from "fs";

// Read the app version from package.json at build time so the About page (and
// anywhere else) reflects the real release version automatically — bumped by
// bump-version.* alongside the server/Android versions. No more stale "v1.0.0".
const pkg = JSON.parse(
  readFileSync(path.resolve(__dirname, "package.json"), "utf-8"),
) as { version: string };

export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "src"),
    },
  },
  server: {
    proxy: {
      "/api": "http://localhost:8080",
      "/health": "http://localhost:8080",
    },
  },
});
