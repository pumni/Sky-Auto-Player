import babel from '@rolldown/plugin-babel';
import tailwindcss from '@tailwindcss/vite';
import react, { reactCompilerPreset } from '@vitejs/plugin-react';
import { defineConfig } from 'vite';
import { fileURLToPath } from 'node:url';

const srcDirectory = fileURLToPath(new URL('./src', import.meta.url));

export default defineConfig({
  plugins: [tailwindcss(), react(), babel({ presets: [reactCompilerPreset()] })],
  resolve: {
    alias: { '@': srcDirectory },
  },
  server: { host: '127.0.0.1', port: 1420, strictPort: true },
  clearScreen: false,
});
