import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // The core's build directory is not renderer source, and watching it makes the dev
    // server die with EBUSY the moment cargo rewrites a locked artifact.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG ? false : "oxc",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    coverage: {
      // Everything the renderer ships, so a module with no test at all is counted as the
      // zero it is rather than left out of the total.
      include: ["src/**/*.{ts,tsx}"],
      // The entry point mounts React and the components are drawn rather than computed;
      // neither is covered by the unit tests, and counting them only hides which of the
      // modules that are testable have no test.
      exclude: ["src/main.tsx", "src/components/**"],
      reporter: ["text"],
    },
  },
});
