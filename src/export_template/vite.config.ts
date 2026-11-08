import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'vite'
import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import { nitro } from 'nitro/vite'
import react from '@vitejs/plugin-react'
import { fileURLToPath } from 'node:url'

export default defineConfig({
  plugins: [tailwindcss(), tanstackStart(), nitro({ preset: 'bun' }), react()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  build: { outDir: '.mochi/dist' },
  // Compatibility styles use .css?inline per document. New .module.css files
  // use Vite's normal local scoping, so authored components can migrate safely.
  server: { host: '127.0.0.1' },
})
