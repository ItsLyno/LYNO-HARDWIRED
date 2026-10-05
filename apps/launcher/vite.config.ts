import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed port and doesn't need Vite to clear the terminal. IPv4:
// on Windows `localhost` is ::1 for Vite, while `tauri dev` waits on 127.0.0.1.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { host: "127.0.0.1", port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
});
