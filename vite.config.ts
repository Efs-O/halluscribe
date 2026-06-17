/// <reference types="vitest" />
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    host: "0.0.0.0",
    port: 6420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/target/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    includeSource: ["src/**/*.ts"],
  },
});
