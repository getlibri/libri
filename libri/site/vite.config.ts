import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// `npm run dev` proxies the API to a Libri serving on this address, or on
// LIBRI_DEV_TARGET's.
export default defineConfig(({ mode }) => {
  const libri = loadEnv(mode, ".", "LIBRI_").LIBRI_DEV_TARGET || "http://127.0.0.1:8950";
  return {
    plugins: [react(), tailwindcss()],
    // Relative, as are the app's API paths, so one build serves both at the
    // root and under /@<unit>/ (the split view's panes).
    base: "./",
    // The same host serves the assistant's pages by name, so the build's
    // files sit under a name show_page refuses.
    // Two pages: the app, and a whiteboard alone (board.html), which loads
    // only the board and its editor for the Android app's place of one and
    // for board links.
    build: { assetsDir: "_site", rollupOptions: { input: { main: "index.html", board: "board.html" } } },
    server: {
      proxy: {
        // ws: the terminal's socket.
        "/api": { target: libri, ws: true },
        "/login": libri,
        // As nginx sends /@<unit>/ to one assistant; here all go to the one.
        "^/@[^/]+/(api|login)": { target: libri, rewrite: (p) => p.replace(/^\/@[^/]+/, "") },
      },
    },
  };
});
