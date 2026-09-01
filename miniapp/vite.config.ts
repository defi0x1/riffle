import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Mini App bundle: served over HTTPS from the domain registered with BotFather.
// Kept deliberately close to Vite defaults -- no proxy, no dev-only bypass of
// production behaviour, since the build served to a real Telegram client should
// match what runs in `npm run dev` as closely as possible.
export default defineConfig({
  plugins: [react()],
  build: {
    target: "es2022",
    sourcemap: true,
  },
  server: {
    host: true,
    port: 5173,
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
  },
});
