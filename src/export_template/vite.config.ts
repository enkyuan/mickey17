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
  // 3000 is the TanStack Start default for `bun run dev` and for the Nitro server
  // behind `bun start`. `PORT=4000 bun run dev` overrides the dev port; strictPort
  // fails loudly on a conflict instead of silently moving to another port.
  server: { host: '127.0.0.1', port: Number(process.env.PORT) || 3000, strictPort: true },
})
