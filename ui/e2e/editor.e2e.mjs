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
    assert.equal(await page.locator('.palette').count(), 0)
    assert.equal(await page.locator('.inspector-panel').count(), 0)
    const file = page.locator('[role="treeitem"][data-path="services/orders/src/flows/rpc.rs"]')
    await file.click()
    assert.match(await page.locator('.code-main pre').innerText(), /pub async fn create_order/)

    // Folders collapse and expand, from the mouse and from the keyboard.
    const dir = page.locator('[role="treeitem"][data-path="services/orders/src/flows"]')
    await dir.click()
    assert.equal(await file.count(), 0)
    await dir.focus()
    await page.keyboard.press('ArrowRight')
    assert.equal(await file.count(), 1)
    assert.equal(await dir.getAttribute('aria-expanded'), 'true')
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

  it('makes Notifications consume orders.created by dragging an edge from the topic', async () => {
    const { page } = await open()
    const edges = await page.locator('.react-flow__edge').count()
    const from = page.locator('.react-flow__node', { hasText: 'orders.created' }).locator('.react-flow__handle.source')
    const to = page.locator('.react-flow__node', { hasText: 'Notifications' }).locator('.react-flow__handle[data-handleid="in"]')
    await from.dragTo(to)
    await settle(page)
    assert.equal(await page.locator('.react-flow__edge').count(), edges + 1)
    assert.equal(await page.getByLabel('Nom du service').inputValue(), 'Notifications')
    assert.ok((await page.locator('.card.consumer').allInnerTexts()).some((t) => t.includes('on_orders_created')))
    assert.equal(await page.locator('.status').innerText(), 'Valide')

    // Already consumed: a second drag adds nothing.
    await from.dragTo(to)
    await settle(page)
    assert.equal(await page.locator('.react-flow__edge').count(), edges + 1)
    await page.close()
  })

  it('declares Notifications as a producer of orders.created by dragging an edge from the service', async () => {
    const { page } = await open()
    const edges = await page.locator('.react-flow__edge').count()
    const from = page.locator('.react-flow__node', { hasText: 'Notifications' }).locator('.react-flow__handle.source[data-handleid="out"]')
    const to = page.locator('.react-flow__node', { hasText: 'orders.created' }).locator('.react-flow__handle.target')
    await from.dragTo(to)
    await settle(page)
    assert.equal(await page.locator('.react-flow__edge').count(), edges + 1)
    // Declared but nothing publishes yet: a warning, not an error.
    assert.ok((await page.locator('.problems li').allInnerTexts()).some((t) => t.includes('W_PRODUCER_NEVER_PUBLISHES')))
    await page.locator('.tree button', { hasText: 'orders.created' }).click()
    assert.deepEqual(await page.locator('.producer .link-btn').allTextContents(), ['Orders', 'Notifications'])

    // Drawing it again adds nothing.
    await from.dragTo(to)
    await settle(page)
    assert.equal(await page.locator('.react-flow__edge').count(), edges + 1)
    await page.close()
  })

  it('declares the producer when a Publish step is added, and errors when the declaration is removed', async () => {
    const { page } = await open()
    await page.locator('.node.service', { hasText: 'Notifications' }).click()
    await page.locator('.card.consumer .card-toggle').first().click()
    await page.locator('.add-step-btn').first().click()
    await page.getByRole('menuitem', { name: /Publier un événement/ }).click()
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)
    assert.ok(!(await page.locator('.problems li').allInnerTexts()).some((t) => t.includes('E_NOT_PRODUCER')))
    await page.locator('.tree button', { hasText: 'orders.created' }).click()
    assert.ok((await page.locator('.producer .link-btn').allTextContents()).includes('Notifications'))

    // Orders publishes orders.created: it cannot stop being a producer while it does.
    await page.getByRole('button', { name: 'Retirer Orders des producteurs' }).click()
    await settle(page)
    assert.ok((await page.locator('.problems li').allInnerTexts()).some((t) => t.includes('E_NOT_PRODUCER')))
    assert.ok(await page.getByRole('button', { name: /Exporter le code/ }).isDisabled())
    await page.close()
  })

  it('separates event, proto and API schemas', async () => {
    const { page } = await open()
    const group = (title) => page.getByRole('region', { name: `Schémas ${title}` })
    assert.deepEqual(await group('Événements').locator('button.mono, button .mono').allTextContents(), ['OrderCreated', 'PaymentCompleted', 'PaymentFailed'])
    assert.ok((await group('Proto (gRPC)').innerText()).includes('CreateOrderRequest'))
    assert.ok((await group('API').innerText()).includes('OrderView'))

    // New API schema: it shows up in its own group and is neither a topic payload nor a gRPC message.
    await group('API').getByRole('button', { name: /Ajouter un schéma API/ }).click()
    assert.ok((await group('API').innerText()).includes('NewApiSchema'))
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)
    assert.equal(await page.getByLabel('Type de NewApiSchema').inputValue(), 'api')

    await page.locator('.tree button', { hasText: 'orders.created' }).click()
    const topicSchemas = await page.getByLabel('Message').locator('option').allTextContents()
    assert.deepEqual(topicSchemas, ['OrderCreated', 'PaymentCompleted', 'PaymentFailed'])

    // Turning the payload of a topic into a proto schema is reported.
    await page.locator('.tree button', { hasText: 'OrderCreated' }).click()
    await page.getByLabel('Type de OrderCreated').selectOption('proto')
    await settle(page)
    assert.ok((await page.locator('.problems li').allInnerTexts()).some((t) => t.includes('E_TOPIC_MESSAGE_KIND')))
    assert.ok(await page.getByRole('button', { name: /Exporter le code/ }).isDisabled())
    await page.close()
  })

  it('opens the endpoint configurator in place of the canvas', async () => {
    const { page } = await open()
    const orders = page.locator('.node.service', { hasText: 'Orders' })
    assert.equal(await orders.locator('.route').count(), 2)

    await orders.locator('.route', { hasText: '/orders' }).first().click()
    await page.locator('.endpoint').waitFor()
    assert.equal(await page.locator('.react-flow').count(), 0, 'the canvas is replaced')
    assert.equal(await page.locator('.inspector-panel').count(), 0)
    assert.equal(await page.getByLabel('Méthode de place_order').inputValue(), 'post')
    assert.equal(await page.getByLabel('Chemin de place_order').inputValue(), '/orders')

    // Two columns: the logic, and the JSON it receives and answers.
    assert.ok(await page.getByRole('region', { name: 'Logique' }).locator('.card.step').count() >= 3)
    const request = await page.getByLabel('Exemple de requête').innerText()
    assert.match(request, /"amount": 0/)
    assert.match(request, /"items": \[\s*"string"\s*\]/)
    assert.match(await page.getByLabel('Exemple de réponse').innerText(), /"order_id": "string"/)

    // Back to the architecture.
    await page.getByRole('button', { name: /Architecture/ }).first().click()
    await page.locator('.react-flow').waitFor()
    assert.equal(await page.locator('.endpoint').count(), 0)
    await page.close()
  })

  it('defines a new route in the configurator: path parameters, schemas and the JSON they make', async () => {
    const { page } = await open()
    await page.locator('.node.service', { hasText: 'Orders' }).click()
    await page.getByRole('button', { name: 'Ajouter une route HTTP' }).click()
    await page.locator('.endpoint').waitFor() // creating a route opens it
    assert.equal(await page.getByLabel('Chemin de new_route').inputValue(), '/new-route')

    // A path parameter that its request schema does not have.
    await page.getByLabel('Chemin de new_route').fill('/things/{thing_id}')
    await page.getByLabel('Chemin de new_route').press('Enter')
    await settle(page)
    assert.ok((await page.locator('.ep-problems').innerText()).includes('E_ROUTE_PARAM'))
    assert.ok(await page.getByRole('button', { name: /Exporter le code/ }).isDisabled())

    await page.getByLabel('Chemin de new_route').fill('/things/{id}')
    await page.getByLabel('Chemin de new_route').press('Enter')
    await page.getByLabel('Méthode de new_route').selectOption('put')
    // A PUT carries the other fields in a JSON body: add one and see it in the example.
    await page.getByRole('article', { name: 'Requête' }).getByRole('button', { name: '+ Ajouter un champ' }).click()
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)
    assert.match(await page.getByLabel('Exemple de requête').innerText(), /"field_2": "string"/)
    assert.doesNotMatch(await page.getByLabel('Exemple de requête').innerText(), /"id"/, 'the path parameter is not in the body')

    // The route is listed under its service in the tree, and in the generated code.
    assert.ok((await page.locator('.tree button.sub').allInnerTexts()).some((t) => t.includes('/things/{id}')))
    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.locator('[role="treeitem"][data-path="services/orders/src/transport/http.rs"]').click()
    const code = await page.locator('.code-main pre').innerText()
    assert.match(code, /"\/things\/\{id\}"/)
    assert.match(code, /put\(handle_new_route\)/)
    await page.close()
  })

  it('builds the response with the logic: a schema variable filled in Rust, then read field by field', async () => {
    const { page } = await open()
    await page.locator('.tree button.sub', { hasText: '/orders/{order_id}' }).click()
    await page.getByRole('button', { name: /Construire la réponse par la logique/ }).click()
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)
    // The variable shows in the logic, and every response field now reads it.
    assert.ok((await page.getByRole('region', { name: 'Logique' }).innerText()).includes('result'))
    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.locator('[role="treeitem"][data-path="services/orders/src/flows/api.rs"]').click()
    const code = await page.locator('.code-main pre').innerText()
    assert.match(code, /let result: pb::OrderView = /)
    assert.match(code, /order_id: result\.order_id\.clone\(\)/)
    await page.close()
  })

  it('refuses a request with a status: only HTTP routes offer it', async () => {
    const { page } = await open()
    // A gRPC handler answers with a response, not an HTTP status.
    await page.locator('.node.service', { hasText: 'Orders' }).click()
    await page.locator('.card.rpc .card-toggle', { hasText: 'CreateOrder' }).click()
    await page.locator('.add-step-btn').first().click()
    assert.equal(await page.getByRole('menuitem', { name: /Refuser/ }).count(), 0)
    await page.locator('.add-step-btn').first().click()

    // The sample's `place_order` already refuses a non-positive amount.
    await page.locator('.tree button.sub', { hasText: '/orders/{order_id}' }).click()
    await page.locator('.endpoint').waitFor()
    await page.locator('.add-step-btn').first().click()
    await page.getByRole('menuitem', { name: /Refuser/ }).click()
    const step = page.locator('.card.step', { hasText: 'Refuser' })
    assert.match(await step.innerText(), /400 bad request/)
    assert.equal(await step.getByLabel('Réessayer après (secondes)').count(), 0)

    // 429 asks how long to wait; any other status takes it away again.
    await step.getByLabel('Statut').selectOption('429')
    await step.getByLabel('Message').fill('slow down')
    await step.getByLabel('Réessayer après (secondes)').waitFor()
    await step.getByLabel('Statut').selectOption('404')
    assert.equal(await step.getByLabel('Réessayer après (secondes)').count(), 0)
    await step.getByLabel('Statut').selectOption('429')
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)

    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.locator('[role="treeitem"][data-path="services/orders/src/flows/api.rs"]').click()
    const code = await page.locator('.code-main pre').innerText()
    assert.match(code, /HttpError::new\(429, "slow down"\)/)
    assert.match(code, /HttpError::new\(400, "the amount must be positive"\)/)
    await page.close()
  })

  it('declares uuid, optional and list variables', async () => {
    const { page } = await open()
    await page.locator('.tree button.sub', { hasText: '/orders/{order_id}' }).click()
    await page.locator('.endpoint').waitFor()
    const add = async () => {
      await page.locator('.add-step-btn').first().click()
      await page.getByRole('menuitem', { name: /Variable/ }).click()
    }
    const card = (n) => page.locator('.card.step', { hasText: 'Variable' }).nth(n)

    // A uuid starts on a valid (nil) value; a malformed one is caught.
    await add()
    await card(0).getByLabel('Nom de la variable').fill('trace')
    await card(0).getByLabel('Type', { exact: true }).selectOption('uuid')
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)
    await card(0).getByLabel('Valeur : valeur').fill('not-a-uuid')
    await settle(page)
    assert.match(await page.locator('.status').innerText(), /erreur/)
    await card(0).getByLabel('Valeur : valeur').fill('123e4567-e89b-12d3-a456-426614174000')

    // A list: its value is a JSON array, applied once it parses.
    await add()
    await card(1).getByLabel('Nom de la variable').fill('tags')
    await card(1).getByLabel('Liste').check()
    await card(1).getByLabel('Valeur : valeur').fill('["a", "b"]')
    await card(1).getByLabel('Valeur : valeur').fill('["a", "b"')
    assert.equal(await card(1).getByLabel('Valeur : valeur').getAttribute('aria-invalid'), 'true')
    await card(1).getByLabel('Valeur : valeur').fill('["a", "b"]')
    assert.equal(await card(1).getByLabel('Optionnel').isDisabled(), true)

    // An optional value starts present, and can be left out.
    await add()
    await card(2).getByLabel('Nom de la variable').fill('limit')
    await card(2).getByLabel('Type', { exact: true }).selectOption('int64')
    await card(2).getByLabel('Optionnel').check()
    await card(2).getByLabel('Valeur : absent').check()
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)

    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.locator('[role="treeitem"][data-path="services/orders/src/flows/api.rs"]').click()
    const code = await page.locator('.code-main pre').innerText()
    assert.match(code, /let trace: String = "123e4567-e89b-12d3-a456-426614174000"\.to_string\(\);/)
    assert.match(code, /let tags: Vec<String> = vec!\["a"\.to_string\(\), "b"\.to_string\(\)\];/)
    assert.match(code, /let limit: Option<i64> = None;/)
    await page.close()
  })

  it('hides events, proto schemas and topics while an endpoint is open', async () => {
    const { page } = await open()
    const tree = page.locator('.tree')
    assert.match(await tree.innerText(), /Topics/)
    assert.match(await tree.innerText(), /Événements/)
    await page.locator('.tree button.sub', { hasText: '/orders/{order_id}' }).click()
    await page.locator('.endpoint').waitFor()
    const apiTree = await tree.innerText()
    for (const hidden of ['Topics', 'Événements', 'Proto', 'orders.created', 'OrderCreated', 'Customer']) assert.ok(!apiTree.includes(hidden), `${hidden} should be hidden`)
    assert.ok(apiTree.includes('OrderView'), 'API schemas stay')
    assert.ok(apiTree.includes('/orders/{order_id}'), 'services and routes stay')
    assert.equal(await page.locator('.tile').count(), 0, 'the architecture tiles are out of the way too')
    await page.getByRole('button', { name: /Architecture/ }).first().click()
    assert.match(await page.locator('.tree').innerText(), /Topics/)
    await page.close()
  })

  it('designs tables in their own view', async () => {
    const { page } = await open()
    await page.getByRole('tab', { name: /Tables/ }).click()
    await page.getByTestId('tables-view').waitFor()
    assert.equal(await page.locator('.palette').count(), 0)
    assert.equal(await page.locator('.inspector-panel').count(), 0)
    // The sample's table of Orders is there, with its generated SQL.
    assert.equal(await page.getByLabel('Nom de la table').inputValue(), 'orders')
    assert.match(await page.getByLabel('SQL de la table').innerText(), /PRIMARY KEY \(order_id\)/)
    assert.equal(await page.getByLabel('Nom de la colonne').count(), 4)

    // A new column, nullable until stated otherwise; then its name, its type, its default.
    await page.getByRole('button', { name: '+ Ajouter une colonne' }).click()
    await page.getByLabel('Nom de la colonne').nth(4).fill('discount')
    await page.getByLabel('Nom de la colonne').nth(4).press('Enter')
    await page.getByLabel('Type de la colonne').nth(4).selectOption('double')
    await page.getByLabel('Valeur par défaut').nth(4).fill('0')
    await page.getByLabel('Valeur par défaut').nth(4).press('Enter')
    await settle(page)
    assert.match(await page.getByLabel('SQL de la table').innerText(), /discount DOUBLE PRECISION DEFAULT 0/)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)

    // A table for another service.
    await page.getByRole('region', { name: 'Tables de Payments' }).getByRole('button', { name: /Ajouter une table/ }).click()
    await page.waitForTimeout(250)
    assert.equal(await page.getByLabel('Nom de la table').inputValue(), 'new_table')
    await settle(page)

    // The table shows up in the generated code, additively.
    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.locator('[role="treeitem"][data-path="services/orders/src/app/schema.rs"]').click()
    const code = await page.locator('.code-main pre').innerText()
    assert.match(code, /ADD COLUMN IF NOT EXISTS .{1,4}discount.{1,4} DOUBLE PRECISION DEFAULT 0/) // nullable: it can join a table that has rows
    await page.close()
  })

  it('designs jsonb columns, UNIQUE constraints, indexes and composite foreign keys', async () => {
    const { page } = await open()
    await page.getByRole('tab', { name: /Tables/ }).click()
    await page.getByTestId('tables-view').waitFor()
    const sql = () => page.getByLabel('SQL de la table').innerText()

    // A jsonb column.
    await page.getByRole('button', { name: '+ Ajouter une colonne' }).click()
    await page.getByLabel('Nom de la colonne').nth(4).fill('meta')
    await page.getByLabel('Nom de la colonne').nth(4).press('Enter')
    await page.getByLabel('Type de la colonne').nth(4).selectOption('jsonb')
    assert.match(await sql(), /meta JSONB/)

    // A column that no step reads, so that renaming it later keeps the project valid.
    await page.getByRole('button', { name: '+ Ajouter une colonne' }).click()
    await page.getByLabel('Nom de la colonne').nth(5).fill('region')
    await page.getByLabel('Nom de la colonne').nth(5).press('Enter')

    // A UNIQUE constraint on two columns: it starts on the first one, the order is the order of ticking.
    await page.getByRole('button', { name: '+ Ajouter une contrainte' }).click()
    await page.getByLabel('Colonnes uniques : region').check()
    assert.match(await sql(), /ADD CONSTRAINT orders_order_id_uq UNIQUE \(order_id, region\)/)

    // An index, unique this time.
    await page.getByRole('button', { name: '+ Ajouter un index' }).click()
    await page.getByLabel('Colonnes de l’index : created_at').check()
    await page.getByLabel('Colonnes de l’index : order_id').uncheck()
    await page.getByLabel('est unique').check()
    assert.match(await sql(), /CREATE UNIQUE INDEX IF NOT EXISTS orders_order_id_idx ON orders \(created_at\)/)

    // A second table, with a foreign key on (order_id, region): allowed by the UNIQUE constraint above.
    await page.getByRole('region', { name: 'Tables de Orders' }).getByRole('button', { name: /Ajouter une table/ }).click()
    await page.getByRole('button', { name: '+ Ajouter une colonne' }).click()
    await page.getByLabel('Nom de la colonne').nth(1).fill('order_id')
    await page.getByLabel('Nom de la colonne').nth(1).press('Enter')
    await page.getByRole('button', { name: '+ Ajouter une colonne' }).click()
    await page.getByLabel('Nom de la colonne').nth(2).fill('region')
    await page.getByLabel('Nom de la colonne').nth(2).press('Enter')
    await page.getByRole('button', { name: '+ Ajouter une clé étrangère' }).click()
    assert.equal(await page.getByLabel('Table référencée').inputValue(), 'orders')
    // The column of the same name is picked; a second pair is added by hand.
    assert.equal(await page.getByLabel('Colonne 1', { exact: true }).inputValue(), 'order_id')
    await page.getByRole('button', { name: '+ Ajouter une paire de colonnes' }).click()
    await page.getByLabel('Colonne 2', { exact: true }).selectOption('region')
    await page.getByLabel('Colonne référencée 2').selectOption('region')
    await page.getByLabel('À la suppression').selectOption('cascade')
    await settle(page)
    assert.match(await sql(), /FOREIGN KEY \(order_id, region\) REFERENCES orders \(order_id, region\) ON DELETE CASCADE ON UPDATE NO ACTION/)
    assert.equal(await page.getByLabel('Problèmes de la contrainte').count(), 0)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)

    // Pairing a text column with a number is caught, under the foreign key.
    await page.getByLabel('Colonne référencée 2').selectOption('amount')
    await settle(page)
    assert.match(await page.getByLabel('Problèmes de la contrainte').innerText(), /same type|même type/)
    await page.getByLabel('Colonne référencée 2').selectOption('region')
    await settle(page)
    assert.equal(await page.getByLabel('Problèmes de la contrainte').count(), 0)

    // Renaming a column follows it in the constraints of the other table, and the table is renamed too.
    await page.getByRole('region', { name: 'Tables de Orders' }).getByRole('button', { name: 'orders', exact: true }).click()
    await page.getByLabel('Nom de la colonne').nth(5).fill('zone')
    await page.getByLabel('Nom de la colonne').nth(5).press('Enter')
    await page.getByRole('region', { name: 'Tables de Orders' }).getByRole('button', { name: 'new_table', exact: true }).click()
    assert.match(await sql(), /REFERENCES orders \(order_id, zone\)/)
    await settle(page)

    // All of it reaches the generated schema, after the tables.
    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.locator('[role="treeitem"][data-path="services/orders/src/app/schema.rs"]').click()
    const code = await page.locator('.code-main pre').innerText()
    assert.match(code, /JSONB/)
    assert.match(code, /CREATE UNIQUE INDEX IF NOT EXISTS .{1,4}orders_order_id_idx/)
    // The referencing column keeps its name: only the referenced side follows the rename.
    assert.match(code, /FOREIGN KEY \(.{1,4}order_id.{1,4}, .{1,4}region.{1,4}\) REFERENCES .{1,4}orders.{1,4} \(.{1,4}order_id.{1,4}, .{1,4}zone.{1,4}\)/)
    await page.close()
  })

  it('types the result of a SQL block after the ticked columns', async () => {
    const { page } = await open()
    await page.locator('.tree button.sub', { hasText: '/orders/{order_id}' }).click()
    const sql = page.getByRole('region', { name: 'Logique' }).locator('.card.step', { hasText: 'SQL' }).first()
    assert.equal(await page.getByLabel('Table', { exact: true }).inputValue(), 'orders')
    assert.match(await page.getByLabel('Requête SQL').innerText(), /SELECT order_id, status FROM orders WHERE order_id = input.order_id LIMIT 1/)
    // The variable and the fields the next steps can read.
    const prediction = await page.getByLabel('Variable produite').innerText()
    assert.match(prediction, /order\.order_id/)
    assert.match(prediction, /order\.status/)
    assert.doesNotMatch(prediction, /order\.amount/)

    // Ticking a column adds it to what the next steps can read.
    await page.locator('fieldset.columns label', { hasText: 'amount' }).locator('input').check()
    assert.match(await page.getByLabel('Variable produite').innerText(), /order\.amount/)

    // Unticking the column that the answer reads is caught: the result no longer has it.
    await page.locator('fieldset.columns label', { hasText: 'status' }).locator('input').uncheck()
    await settle(page)
    assert.doesNotMatch(await page.getByLabel('Variable produite').innerText(), /order\.status/)
    assert.ok((await page.locator('.problems li').allInnerTexts()).some((t) => t.includes('E_UNKNOWN_PATH')))
    await page.locator('fieldset.columns label', { hasText: 'status' }).locator('input').check()
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)
    assert.ok(await sql.count() >= 1)
    await page.close()
  })

  it('adds a SQL block that starts from the key the request carries', async () => {
    const { page } = await open()
    await page.locator('.tree button.sub', { hasText: '/orders/{order_id}' }).click()
    await page.getByRole('region', { name: 'Logique' }).locator('.add-step-btn').click()
    await page.getByRole('menuitem', { name: /SQL/ }).click()
    // A second read of the same table: its variable gets its own name, and the key is already matched.
    const preview = page.getByLabel('Requête SQL').last()
    assert.match(await preview.innerText(), /WHERE order_id = input\.order_id/)
    assert.match(await page.getByLabel('Variable produite').last().innerText(), /order2\.order_id/)
    await settle(page)
    assert.doesNotMatch(await page.locator('.status').innerText(), /erreur/)

    // An update without a condition is refused: it would touch every row.
    await page.getByRole('button', { name: 'Modifier', exact: true }).last().click()
    await page.locator('.sql-condition').last().getByRole('button', { name: 'Retirer la condition' }).click()
    await settle(page)
    assert.ok((await page.locator('.problems li').allInnerTexts()).some((t) => t.includes('E_SQL_NO_WHERE')))
    await page.close()
  })

  it('keeps the condition block stacked, with the SQL block’s own condition rows next to it', async () => {
    const { page } = await open()
    await page.locator('.tree button.sub', { hasText: '/orders/{order_id}' }).click()
    const logic = page.getByRole('region', { name: 'Logique' })
    await logic.locator('.add-step-btn').click()
    await page.getByRole('menuitem', { name: /Condition/ }).click()
    const editor = page.locator('.stack.condition').last()
    await editor.waitFor()
    // “Si” + type, then the left operand, the operator and the right operand, one under the other.
    const tops = await editor.evaluate((el) => [...el.children].map((c) => Math.round(c.getBoundingClientRect().top)))
    assert.ok(tops.length >= 3, `expected the rows of the editor, got ${tops.length}`)
    assert.deepEqual(tops, [...tops].sort((a, b) => a - b))
    assert.equal(new Set(tops).size, tops.length, `rows overlap on one line: ${tops}`)
    assert.equal(await editor.evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length), 1)

    // And the SQL block, which shares the word “condition”, keeps its own four-column rows.
    await logic.locator('.add-step-btn').last().click() // the branch added two nested lists, the handler's own is last
    await page.getByRole('menuitem', { name: /SQL/ }).click()
    const row = page.locator('.sql-condition').last()
    assert.equal(await row.evaluate((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length), 4)
    await page.close()
  })

  it('sets the outbox attempts for the whole project and generates them into the services', async () => {
    const { page } = await open()
    await page.getByLabel('Tentatives avant DLQ').fill('8')
    await settle(page)
    assert.equal(await page.locator('.status').innerText(), 'Valide')
    await page.getByLabel('Tentatives avant DLQ').fill('0') // out of range: clamped to 1
    await settle(page)
    assert.equal(await page.getByLabel('Tentatives avant DLQ').inputValue(), '1')
    assert.equal(await page.locator('.status').innerText(), 'Valide')
    await page.getByLabel('Tentatives avant DLQ').fill('8')
    await settle(page)

    await page.getByRole('tab', { name: /Code généré/ }).click()
    await page.locator('[role="treeitem"][data-path="services/orders/src/app/config.rs"]').click()
    assert.match(await page.locator('.code-main pre').innerText(), /OUTBOX_MAX_ATTEMPTS", "8"/)
    await page.locator('[role="treeitem"][data-path="services/orders/src/transport/outbox.rs"]').click()
    assert.match(await page.locator('.code-main pre').innerText(), /Relay::new/)
    await page.close()
  })

  it('keeps the project valid when a topic and a message are renamed', async () => {
    const { page } = await open()
    await page.locator('.tree button', { hasText: 'orders.created' }).click()
    await page.getByLabel('Nom du topic').fill('orders.accepted')
    await page.getByLabel('Nom du topic').press('Enter')
    await page.locator('.tree button', { hasText: 'OrderCreated' }).click()
    await page.getByLabel('Nom du schéma').fill('OrderAccepted')
    await page.getByLabel('Nom du schéma').press('Enter')
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
    assert.ok(fs.existsSync(path.join(out, 'services/orders/src/flows/rpc.rs')))
    await page.close()
    fs.rmSync(tmp, { recursive: true, force: true })
  })
})

describe('auto-update', () => {
  let browser
  before(async () => (browser = await launchBrowser()))
  after(async () => browser?.close())

  const open = async (opts) => {
    const app = await openApp(browser, URL, opts)
    await app.page.waitForTimeout(400)
    return app
  }
  const button = (page) => page.getByRole('button', { name: 'Mettre à jour' })

  it('checks the release feed at launch and announces the new version on the button', async () => {
    const { page, calls, errors } = await open({ update: { version: '0.2.0' } })
    assert.deepEqual(calls, ['plugin:updater|check'])
    assert.match(await button(page).innerText(), /v0\.2\.0/)
    assert.equal(await button(page).isEnabled(), true)
    assert.deepEqual(errors, [])
    await page.close()
  })

  it('keeps a plain button when the app is current and says so on click', async () => {
    const { page, calls } = await open()
    assert.equal(await button(page).getAttribute('title'), 'Rechercher une nouvelle version')
    await button(page).click()
    await page.waitForSelector('.toast.info')
    assert.match(await page.locator('.toast').innerText(), /à jour \(v0\.1\.0\)/)
    assert.deepEqual(calls, ['plugin:updater|check', 'plugin:updater|check'])
    await page.close()
  })

  it('stays silent at launch when the feed is unreachable, reports it when asked', async () => {
    const { page } = await open({ updateFails: true })
    assert.equal(await page.locator('.toast').count(), 0)
    await button(page).click()
    await page.waitForSelector('.toast.error')
    assert.match(await page.locator('.toast').innerText(), /Recherche de mise à jour impossible/)
    await page.close()
  })

  it('installs the update and restarts the app when clicked', async () => {
    const { page, calls, asked } = await open({ update: { version: '0.2.0' } })
    await button(page).click()
    await page.waitForTimeout(300)
    assert.equal(asked.length, 1)
    assert.match(asked[0], /version 0\.2\.0.*redémarrer/)
    assert.deepEqual(calls, ['plugin:updater|check', 'plugin:updater|download_and_install', 'plugin:process|restart'])
    await page.close()
  })

  it('sits next to “Exporter le code” and fits the toolbar at the minimum width', async () => {
    const { page } = await open({ update: { version: '0.2.0' }, viewport: { width: 1100, height: 700 } })
    const update = await button(page).boundingBox()
    const exportBtn = await page.getByRole('button', { name: /Exporter le code/ }).boundingBox()
    assert.ok(update.x + update.width <= exportBtn.x + 1, 'update button is left of export')
    assert.ok(exportBtn.x + exportBtn.width <= 1100, 'export button stays inside the window')
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true)
    await page.close()
  })

  it('keeps the project name readable from the default window width down to the minimum', async () => {
    for (const width of [1480, 1300, 1100]) {
      const { page } = await open({ update: { version: '0.2.0' }, viewport: { width, height: 700 } })
      const name = await page.getByLabel('Nom du projet').boundingBox()
      assert.ok(name.width >= 120, `project name is ${Math.round(name.width)}px wide at ${width}px`)
      await page.close()
    }
  })
})

describe('mcp', () => {
  let browser
  let tmp
  before(async () => {
    browser = await launchBrowser()
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'sb-e2e-mcp-'))
  })
  after(async () => {
    await browser?.close()
    fs.rmSync(tmp, { recursive: true, force: true })
  })

  const open = async (opts) => {
    const app = await openApp(browser, URL, opts)
    await app.page.waitForTimeout(400)
    return app
  }
  const tab = (page) => page.getByRole('tab', { name: 'MCP' })

  it('lists the tools and the commands, but asks for a saved project before activating', async () => {
    const { page, errors } = await open()
    await tab(page).click()
    await page.getByRole('list', { name: 'Liste des outils' }).waitFor()
    assert.equal(await page.getByRole('list', { name: 'Liste des outils' }).getByRole('listitem').count(), 2)
    assert.match(await page.getByTestId('copy-text').first().innerText(), /^\/opt\/service-builder\/service-builder mcp '<projet\.sb\.json>'$/)
    assert.match(await page.getByLabel('Activation').innerText(), /Enregistrez le projet/)
    assert.equal(await page.getByRole('button', { name: 'Activer' }).count(), 0)
    assert.equal(await page.locator('.palette').count(), 0)
    assert.deepEqual(errors, [])
    await page.close()
  })

  it('activates and deactivates the server for the saved project', async () => {
    const save = path.join(tmp, 'demo.sb.json')
    const { page, mcpCalls } = await open({ dialogs: { save } })
    await page.getByRole('button', { name: /Enregistrer$/ }).click()
    await page.waitForTimeout(400)
    await tab(page).click()
    await page.getByTestId('mcp-state').waitFor()
    assert.equal(await page.getByTestId('mcp-state').innerText(), 'Inactif')
    assert.match(await page.getByTestId('copy-text').nth(1).innerText(), new RegExp(`^claude mcp add service-builder-demo -- /opt/service-builder/service-builder mcp ${save}$`))

    await page.getByRole('button', { name: 'Activer' }).click()
    await page.locator('[data-testid=mcp-state][data-state=on]').waitFor()
    assert.equal(await page.getByTestId('mcp-state').innerText(), 'Actif')
    assert.match(await page.getByLabel('Activation').innerText(), /\.mcp\.json/)

    await page.getByRole('button', { name: 'Désactiver' }).click()
    await page.locator('[data-testid=mcp-state][data-state=off]').waitFor()
    assert.equal(await page.getByTestId('mcp-state').innerText(), 'Inactif')
    assert.deepEqual(mcpCalls, [`enable ${save}`, `disable ${save}`])
    await page.close()
  })

  it('offers to refresh an entry that starts another program', async () => {
    const save = path.join(tmp, 'stale.sb.json')
    const { page } = await open({ dialogs: { save }, mcpInitial: 'stale' })
    await page.getByRole('button', { name: /Enregistrer$/ }).click()
    await page.waitForTimeout(400)
    await tab(page).click()
    await page.locator('[data-testid=mcp-state][data-state=stale]').waitFor()
    assert.equal(await page.getByTestId('mcp-state').innerText(), 'À mettre à jour')
    assert.equal(await page.getByRole('button', { name: 'Mettre à jour', exact: true }).count() > 0, true)
    await page.close()
  })

  const report = {
    id: 3,
    createdAt: 1790000000,
    summary: 'Hashing passwords blocks the runtime.',
    suggestions: [{ title: 'Blocking task block', details: 'Run code in spawn_blocking.' }],
    status: 'pending',
  }

  it('shows the reports agents filed and opens a pre-filled GitHub issue only on click', async () => {
    const save = path.join(tmp, 'reports.sb.json')
    const { page, opened } = await open({ dialogs: { save }, reports: [report] })
    await page.getByRole('button', { name: /Enregistrer$/ }).click()
    await page.waitForTimeout(400)
    await tab(page).click()
    const card = page.getByLabel('Rapport 3')
    await card.waitFor()
    assert.match(await card.innerText(), /Blocking task block/)
    assert.match(await card.innerText(), /spawn_blocking/)
    assert.deepEqual(opened, [], 'nothing is opened before the click')

    await card.getByRole('button', { name: 'Envoyer sur GitHub' }).click()
    await card.getByText('Ouvert sur GitHub').waitFor()
    assert.deepEqual(opened, ['https://github.com/entcorporg/service-builder/issues/new?title=report-3'])
    assert.match(await card.innerText(), /Ouvert sur GitHub/)
    await page.close()
  })

  it('dismisses a report and can stop the requests altogether', async () => {
    const save = path.join(tmp, 'dismiss.sb.json')
    const { page, feedback } = await open({ dialogs: { save }, reports: [report] })
    await page.getByRole('button', { name: /Enregistrer$/ }).click()
    await page.waitForTimeout(400)
    await tab(page).click()
    await page.getByLabel('Rapport 3').getByRole('button', { name: 'Ignorer' }).click()
    await page.waitForSelector('text=Aucun rapport pour l’instant')
    assert.equal(feedback.reports.length, 0)

    const toggle = page.getByLabel(/Demander un rapport à l’agent tous les 50 appels/)
    assert.equal(await toggle.isChecked(), true)
    await toggle.click() // controlled: it flips once the backend has answered
    await page.waitForFunction(() => [...document.querySelectorAll('label')].find((l) => /Demander un rapport/.test(l.textContent))?.querySelector('input')?.checked === false)
    assert.equal(feedback.enabled, false)
    await page.close()
  })

  it('has no suggestions section before the project is saved', async () => {
    const { page } = await open({ reports: [report] })
    await tab(page).click()
    await page.getByRole('list', { name: 'Liste des outils' }).waitFor()
    assert.equal(await page.getByLabel('Suggestions des agents').count(), 0)
    await page.close()
  })

  it('reloads the project when a client edited the file, unless edits are pending', async () => {
    const save = path.join(tmp, 'live.sb.json')
    const { page, touchFile } = await open({ dialogs: { save } })
    await page.getByRole('button', { name: /Enregistrer$/ }).click()
    await page.waitForTimeout(400)

    // A client renames the project in the file.
    const edit = (name) => {
      const project = JSON.parse(fs.readFileSync(save, 'utf8'))
      fs.writeFileSync(save, JSON.stringify({ ...project, name }))
      touchFile()
    }
    edit('Édité par un client')
    await page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await page.waitForSelector('.toast.info')
    assert.match(await page.locator('.toast').innerText(), /Projet rechargé/)
    assert.equal(await page.getByLabel('Nom du projet').inputValue(), 'Édité par un client')

    // With unsaved edits, the app keeps them and warns instead.
    await page.getByLabel('Nom du projet').fill('Mon brouillon')
    await page.getByLabel('Nom du projet').press('Enter')
    edit('Encore un client')
    await page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await page.waitForFunction(() => /écrasera/.test(document.querySelector('.toast')?.textContent ?? ''))
    assert.equal(await page.getByLabel('Nom du projet').inputValue(), 'Mon brouillon')
    await page.close()
  })
})
