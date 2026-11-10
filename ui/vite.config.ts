import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  build: {
    target: ["es2022", "safari16"],
    chunkSizeWarningLimit: 900,
    sourcemap: false,
  },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
