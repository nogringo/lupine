import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vite";

/** The share links' service worker controls /s/, so it is served from there, under a fixed name. */
const SHARE_WORKER = "s/sw.js";

const shareWorkerInDev: Plugin = {
  name: "lupine-share-worker",
  configureServer(server) {
    server.middlewares.use((request, _, next) => {
      if (request.url === `/${SHARE_WORKER}`) request.url = "/src/share/sw.ts";
      next();
    });
  },
};

export default defineConfig({
  plugins: [react(), shareWorkerInDev],
  // The page that opens share links, at /s/, starts no drive.
  input: { main: "index.html", share: "s/index.html", sw: "src/share/sw.ts" },
  build: {
    rolldownOptions: {
      output: {
        entryFileNames: (chunk) => (chunk.name === "sw" ? SHARE_WORKER : "assets/[name]-[hash].js"),
      },
    },
  },
  // The core's loader finds its .wasm with import.meta.url, which only ES workers keep.
  worker: { format: "es" },
});
