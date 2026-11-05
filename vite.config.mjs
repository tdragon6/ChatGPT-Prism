import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  base: "./",
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  build: {
    target: ["safari16", "edge109"],
    rolldownOptions: {
      input: {
        manager: fileURLToPath(new URL("index.html", import.meta.url)),
        playback: fileURLToPath(new URL("playback.html", import.meta.url)),
      },
    },
  },
});
