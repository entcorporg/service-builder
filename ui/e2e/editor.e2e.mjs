import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { after, before, describe, it } from 'node:test'
import { launchBrowser, openApp } from './harness.mjs'

const URL = process.env.E2E_URL ?? 'http://127.0.0.1:1431/'
const settle = (page) => page.waitForTimeout(800) // analysis is debounced

describe('editor', () => {
  let browser
  before(async () => (browser = await launchBrowser()))
  after(async () => browser?.close())

  const open = async (opts) => {
    const app = await openApp(browser, URL, opts)
    await app.page.waitForTimeout(400)
    return app
  }

  it('boots on the sample project: 6 nodes, 7 derived edges, valid, no console error', async () => {
    const { page, errors } = await open()
    assert.equal(await page.locator('.react-flow__node').count(), 6)
    assert.equal(await page.locator('.react-flow__edge').count(), 7)
    assert.equal(await page.locator('.status').innerText(), 'Valide')
    assert.deepEqual(errors, [])
    await page.close()
  })

  it('shows a service’s RPC steps and the generated Rust', async () => {
    const { page } = await open()
    await page.locator('.node.service', { hasText: 'Orders' }).click()
    assert.equal(await page.getByLabel('Nom du service').inputValue(), 'Orders')
    await page.locator('.card.rpc .card-toggle', { hasText: 'CreateOrder' }).click()
    assert.equal(await page.locator('.card.step').count(), 4)

    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.waitForSelector('.code-view pre')
    const dir = page.locator('.code-tree > div', { has: page.locator('h4', { hasText: /^services\/orders\/src$/ }) })
    await dir.getByRole('button', { name: /flows.rs/ }).click()
    assert.match(await page.locator('.code-main pre').innerText(), /pub async fn create_order/)
    await page.close()
  })

  it('shows nested step lists for a condition', async () => {
    const { page } = await open()
    await page.locator('.node.service', { hasText: 'Payments' }).click()
    await page.waitForSelector('.branch')
    assert.deepEqual((await page.locator('.branch h4').allTextContents()).map((t) => t.toLowerCase()), ['alors', 'sinon'])
    await page.close()
  })

  it('turns red on an invalid step, blocks export, and recovers when it is removed', async () => {
    const { page } = await open()
    await page.locator('.node.service', { hasText: 'Orders' }).click()
    await page.locator('.card.rpc .card-toggle', { hasText: 'CreateOrder' }).click()
    await page.locator('.add-step-btn').first().click()
    await page.getByRole('menuitem', { name: /Appeler un RPC/ }).click() // nothing to call yet: unknown service
    await settle(page)
    assert.match(await page.locator('.status').innerText(), /erreur/)
    assert.ok(await page.getByRole('button', { name: /Exporter le code/ }).isDisabled())
    assert.ok((await page.locator('.problems li').allInnerTexts()).some((t) => t.includes('E_UNKNOWN_SERVICE')))

    await page.locator('.card.step', { hasText: 'Appeler un RPC' }).getByRole('button', { name: 'Supprimer l’étape' }).click()
    await settle(page)
    assert.equal(await page.locator('.status').innerText(), 'Valide')
    await page.close()
  })

  it('creates a topic by dragging it from the palette onto the canvas', async () => {
    const { page } = await open()
    const before = await page.locator('.react-flow__node').count()
    await page.locator('.tile.topic').dragTo(page.locator('.react-flow'), { targetPosition: { x: 300, y: 650 } })
    await page.waitForTimeout(300)
    assert.equal(await page.locator('.react-flow__node').count(), before + 1)
    assert.equal(await page.getByLabel('Nom du topic').inputValue(), 'new.topic')
    await page.close()
  })

  it('keeps the project valid when a topic and a message are renamed', async () => {
    const { page } = await open()
    await page.locator('.tree button', { hasText: 'orders.created' }).click()
    await page.getByLabel('Nom du topic').fill('orders.accepted')
    await page.getByLabel('Nom du topic').press('Enter')
    await page.locator('.tree button', { hasText: 'OrderCreated' }).click()
    await page.getByLabel('Nom du message').fill('OrderAccepted')
    await page.getByLabel('Nom du message').press('Enter')
    await settle(page)
    assert.equal(await page.locator('.status').innerText(), 'Valide')
    assert.match(await page.locator('.node.topic', { hasText: 'orders.accepted' }).innerText(), /OrderAccepted/)
    await page.close()
  })

  it('saves the project and exports the workspace through the native dialogs', async () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'sb-e2e-'))
    const save = path.join(tmp, 'demo.sb.json')
    const out = path.join(tmp, 'out', 'workspace')
    const { page } = await open({ dialogs: { save, open: out } })
    await page.locator('.tree button', { hasText: 'orders.created' }).click()
    await page.getByLabel('Nom du topic').fill('orders.accepted')
    await page.getByLabel('Nom du topic').press('Enter')
    await settle(page)

    await page.getByRole('button', { name: /Enregistrer$/ }).click()
    await page.waitForTimeout(400)
    assert.ok(JSON.parse(fs.readFileSync(save, 'utf8')).topics.some((t) => t.name === 'orders.accepted'))
    assert.equal(await page.locator('.dirty').count(), 0)

    await page.getByRole('button', { name: /Exporter le code/ }).click()
    await page.waitForTimeout(1500)
    assert.match(fs.readFileSync(path.join(out, 'contracts/src/lib.rs'), 'utf8'), /ORDERS_ACCEPTED/)
    assert.ok(fs.existsSync(path.join(out, 'services/orders/src/flows.rs')))
    await page.close()
    fs.rmSync(tmp, { recursive: true, force: true })
  })
})
