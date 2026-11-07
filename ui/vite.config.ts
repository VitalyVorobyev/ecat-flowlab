import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// `bun run dev` serves the UI on :5173 and forwards /api to `flowlab serve` on :7878.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    proxy: { "/api": "http://127.0.0.1:7878" },
  },
  build: { target: ["es2022", "chrome120", "safari17"] },
  test: {
    environment: "happy-dom",
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
