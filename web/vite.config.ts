import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  // The core's loader finds its .wasm with import.meta.url, which only ES workers keep.
  worker: { format: "es" },
});
