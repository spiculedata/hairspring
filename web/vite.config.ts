import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// `npm run dev` proxies API calls to a local `hairspring serve`.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { '/api': 'http://127.0.0.1:7878', '/mcp': 'http://127.0.0.1:7878' } },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false },
  test: { environment: 'node' },
});
