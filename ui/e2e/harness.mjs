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

// `update` is the release the fake feed announces ({ version, body }), or null for an up-to-date app;
// `updateFails` makes the feed unreachable. `calls` lists the updater/process commands the UI issued, `asked` the confirmation questions shown.
export async function openApp(browser, url, { dialogs = {}, viewport = { width: 1480, height: 920 }, update = null, updateFails = false } = {}) {
  const calls = []
  const asked = []
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
      case 'plugin:dialog|message':
        // `ask` is a message with custom buttons; answering with the OK label means “yes”.
        asked.push(args.message)
        return args.buttons?.OkCancelCustom?.[0] ?? 'Yes'
      case 'plugin:app|version':
        return '0.1.0'
      case 'plugin:updater|check':
        calls.push(cmd)
        if (updateFails) throw new Error('feed unreachable')
        return update && { rid: 7, currentVersion: '0.1.0', version: update.version, date: null, body: update.body ?? null, rawJson: {} }
      case 'plugin:updater|download_and_install':
      case 'plugin:process|restart':
        calls.push(cmd)
        return null
      case 'plugin:resources|close':
        return null
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
  return { page, errors, calls, asked }
}
