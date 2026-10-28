// `pnpm e2e`: starts Vite, runs the scenarios in e2e/*.e2e.mjs with node:test, stops Vite.
import { spawn } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const ui = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const port = process.env.E2E_PORT ?? '1431'
const url = `http://127.0.0.1:${port}/`

const vite = spawn('pnpm', ['exec', 'vite', '--host', '127.0.0.1', '--port', port, '--strictPort'], { cwd: ui, stdio: 'ignore' })
const stop = () => vite.kill()
process.on('exit', stop)

for (let i = 0; i < 50; i++) {
  try {
    if ((await fetch(url)).ok) break
  } catch {}
  await new Promise((r) => setTimeout(r, 200))
}

const tests = spawn(process.execPath, ['--test', '--test-concurrency=1', 'e2e/*.e2e.mjs'], {
  cwd: ui,
  stdio: 'inherit',
  env: { ...process.env, E2E_URL: url },
})
tests.on('exit', (code) => {
  stop()
  process.exit(code ?? 1)
})
