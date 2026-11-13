import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // The `util` browser polyfill below reads `process.env.NODE_DEBUG` when it loads, and browsers
  // have no `process`. Without this the whole app crashes with "process is not defined" and the
  // page stays blank. Replacing the constant at build time fixes dev, pre-bundled deps and builds.
  define: {
    'process.env.NODE_DEBUG': 'undefined',
  },
  resolve: {
    alias: {
      '@': '/src',
      // near-api-js uses util.format when generating JSON validators.
      util: 'util/',
    },
  },
})
