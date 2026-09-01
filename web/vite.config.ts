import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // Predictable names: the Rust server reads these off disk on every
    // request, so a stable path is what makes live editing possible.
    rollupOptions: {
      output: {
        entryFileNames: 'assets/app.js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: 'assets/[name][extname]',
      },
    },
  },
  server: {
    port: 5173,
    // In dev, Vite serves the frontend and forwards everything the Rust
    // front door owns. Same URLs in dev and in a build, so nothing is
    // "works only in production".
    proxy: {
      '/api': 'http://127.0.0.1:8888',
      '/r': 'http://127.0.0.1:8888',
    },
  },
})
