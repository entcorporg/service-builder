// Drives the UI in Chromium with a fake Tauri IPC. Every command runs the real Rust code
// through the `sb` CLI (`cargo build -p sb-app --bin sb` first), so the UI is tested against
// the same validator and generator the desktop app uses.
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const SB = process.env.SB_BIN ?? path.join(root, 'target/debug/sb')

function sb(args, input) {
  const r = spawnSync(SB, args, { input, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
  if (r.error) throw new Error(`cannot run ${SB}: ${r.error.message}`)
  return r
}

export async function launchBrowser() {
  // SB_CHROMIUM points at an existing Chromium when Playwright's own download is not available.
  return chromium.launch(process.env.SB_CHROMIUM ? { executablePath: process.env.SB_CHROMIUM } : {})
}

export async function openApp(browser, url, { dialogs = {}, viewport = { width: 1480, height: 920 } } = {}) {
  const page = await browser.newPage({ viewport })
  const errors = []
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()))
  page.on('pageerror', (e) => errors.push(e.message))
  page.on('response', (r) => r.status() >= 400 && errors.push(`${r.status()} ${r.url()}`))

  await page.exposeFunction('__sbInvoke', (cmd, args) => {
    switch (cmd) {
      case 'sample_project':
        return JSON.parse(sb(['sample']).stdout)
      case 'new_project':
        return { schemaVersion: 1, name: args.name, broker: { bootstrapServers: 'localhost:9092' }, messages: [], topics: [], services: [] }
      case 'analyze':
        return JSON.parse(sb(['analyze'], JSON.stringify(args.project)).stdout)
      case 'open_project':
        return JSON.parse(fs.readFileSync(args.path, 'utf8'))
      case 'save_project':
        fs.writeFileSync(args.path, JSON.stringify(args.project, null, 2))
        return null
      case 'export_project': {
        const input = path.join(args.dir, '..', 'export-input.json')
        fs.mkdirSync(path.dirname(input), { recursive: true })
        fs.writeFileSync(input, JSON.stringify(args.project))
        const r = sb(['generate', input, args.dir])
        if (r.status !== 0) throw new Error(r.stderr)
        return { dir: args.dir, files: Number(r.stdout.match(/wrote (\d+)/)[1]) }
      }
      case 'plugin:dialog|open':
        return dialogs.open ?? null
      case 'plugin:dialog|save':
        return dialogs.save ?? null
      case 'plugin:dialog|ask':
        return true
      default:
        throw new Error(`unmocked command ${cmd}`)
    }
  })
  await page.addInitScript(() => {
    let callbacks = 0
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => window.__sbInvoke(cmd, args),
      transformCallback: () => ++callbacks,
      unregisterCallback: () => {},
      convertFileSrc: (p) => p,
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    }
  })
  await page.goto(url)
  await page.waitForSelector('.node.service')
  return { page, errors }
}
