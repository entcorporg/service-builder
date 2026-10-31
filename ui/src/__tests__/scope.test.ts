import { describe, expect, it } from 'vitest'
import { exampleJson, exampleQuery } from '../json'
import { addColumn, addForeignKey, addJob, ensureJwksSchemas, ensurePasskeySchema, ensureTotpSchema, eventKeys, addIndex, addTable, addUnique, makeStep, retargetForeignKey, renameColumn, renameTable, selectionForPath, addMessage, addRoute, addRpc, defaultStatus, pathParams, addTopic, autoMap, canConsume, canProduce, connectConsumer, consumersOf, declareProducer, removeService, renameService, renameField, renameMessage, renameTopic, nestable } from '../model'
import { sqlPreview } from '../components/SqlStep'
import { everyLabel } from '../components/Inspector'
import { createTableSql } from '../components/TablesView'
import { candidates, columnFieldType, columnType, fits, sqlRows, inputScope, scalar, scopeAfter, scopesOf, sqlRowName, typeOfField } from '../scope'
import type { Project } from '../types'

const project = (): Project => ({
  schemaVersion: 3,
  name: 'T',
  broker: { bootstrapServers: 'localhost:9092' },
  outbox: { maxAttempts: 5 },
  messages: [
    { name: 'Customer', kind: 'proto', fields: [{ name: 'name', type: { kind: 'string' }, repeated: false, optional: false }] },
    {
      name: 'Order',
      kind: 'proto',
      fields: [
        { name: 'id', type: { kind: 'string' }, repeated: false, optional: false },
        { name: 'customer', type: { kind: 'message', name: 'Customer' }, repeated: false, optional: false },
        { name: 'note', type: { kind: 'string' }, repeated: false, optional: true },
      ],
    },
    { name: 'Placed', kind: 'event', fields: [{ name: 'id', type: { kind: 'string' }, repeated: false, optional: false }] },
  ],
  topics: [{ name: 'orders.placed', message: 'Placed', partitions: 1, replicationFactor: 1, keyField: 'id', producers: ['Orders'], position: { x: 0, y: 0 } }],
  services: [
    {
      name: 'Orders',
      port: 50051,
      httpPort: null,
      routes: [],
      tables: [],
      jobs: [],
      position: { x: 0, y: 0 },
      rpcs: [
        {
          name: 'Create',
          request: 'Order',
          response: 'Placed',
          steps: [
            { type: 'publish', id: 'a', topic: 'orders.placed', payload: [{ field: 'id', value: { kind: 'path', path: 'input.id' } }] },
            { type: 'respond', id: 'b', fields: [{ field: 'id', value: { kind: 'path', path: 'input.id' } }], status: null, cookies: [] },
          ],
        },
      ],
      consumers: [{ name: 'on_placed', topic: 'orders.placed', groupId: null, idempotentOn: null, steps: [] }],
    },
  ],
})

describe('fits (mirrors ResolvedPath::adapt_to)', () => {
  const string = scalar('string')
  const optString = { ...string, optional: true }
  it('accepts identical types and widening to optional', () => {
    expect(fits(string, false, string)).toBe(true)
    expect(fits(string, false, optString)).toBe(true)
  })
  it('rejects narrowing and different types', () => {
    expect(fits(optString, false, string)).toBe(false)
    expect(fits(string, false, scalar('int64'))).toBe(false)
  })
  it('widens a scalar variable like a field: only a whole message variable is bare', () => {
    // A top-level variable that is a plain string (`let`, a token) is `bare` for `candidates`, yet it fills an optional.
    expect(fits(string, true, optString)).toBe(true)
    expect(fits(string, true, scalar('int64'))).toBe(false)
    const orders = { type: { kind: 'message' as const, name: 'Order' }, repeated: true, optional: false }
    expect(fits(orders, true, orders)).toBe(true)
  })
  it('lets a bare message fill a message field only', () => {
    const order = { type: { kind: 'message' as const, name: 'Order' }, repeated: false, optional: false }
    expect(fits(order, true, order)).toBe(true)
    expect(fits(order, true, string)).toBe(false)
  })
})

describe('candidates and scopes', () => {
  it('lists nested fields with their types, input first', () => {
    const paths = candidates(project(), inputScope('Order')).map((c) => c.path)
    expect(paths).toEqual(['input', 'input.id', 'input.customer', 'input.customer.name', 'input.note'])
  })
  it('binds variables for the steps that follow', () => {
    const p = project()
    const scopes = scopesOf(p, inputScope('Order'), [
      { type: 'let', id: '1', name: 'total', valueType: { kind: 'int64' }, optional: false, repeated: false, value: { kind: 'literal', value: 1 } },
      { type: 'log', id: '2', level: 'info', message: '' },
    ])
    expect(Object.keys(scopes[0]!)).toEqual(['input'])
    expect(Object.keys(scopes[1]!).sort()).toEqual(['input', 'total'])
  })
  it('auto-maps same-named compatible fields only', () => {
    const p = project()
    expect(autoMap(p, inputScope('Order'), 'Placed')).toEqual([{ field: 'id', value: { kind: 'path', path: 'input.id' } }])
    expect(typeOfField(p.messages[1]!.fields[2]!).optional).toBe(true)
  })
})

describe('rename cascades', () => {
  it('renames a topic everywhere it is referenced', () => {
    const p = project()
    renameTopic(p, 'orders.placed', 'orders.accepted')
    expect(p.topics[0]!.name).toBe('orders.accepted')
    expect(p.services[0]!.consumers[0]!.topic).toBe('orders.accepted')
    const publish = p.services[0]!.rpcs[0]!.steps[0]!
    expect(publish.type === 'publish' && publish.topic).toBe('orders.accepted')
  })
  it('renames a message in topics, RPCs and field types', () => {
    const p = project()
    renameMessage(p, 'Customer', 'Buyer')
    expect(p.messages[1]!.fields[1]!.type).toEqual({ kind: 'message', name: 'Buyer' })
    renameMessage(p, 'Placed', 'Accepted')
    expect(p.topics[0]!.message).toBe('Accepted')
    expect(p.services[0]!.rpcs[0]!.response).toBe('Accepted')
  })
  it('renames a field in mappings that target it and in the topic key', () => {
    const p = project()
    renameField(p, 'Placed', 'id', 'order_id')
    expect(p.topics[0]!.keyField).toBe('order_id')
    const publish = p.services[0]!.rpcs[0]!.steps[0]!
    expect(publish.type === 'publish' && publish.payload[0]!.field).toBe('order_id')
    const respond = p.services[0]!.rpcs[0]!.steps[1]!
    expect(respond.type === 'respond' && respond.fields[0]!.field).toBe('order_id')
    // Order.id is a different message: untouched.
    expect(p.messages[1]!.fields[0]!.name).toBe('id')
  })
  it('finds the services consuming a topic', () => {
    const p = project()
    expect(consumersOf(p, 'orders.placed').map((s) => s.name)).toEqual(['Orders'])
    p.services.push({ name: 'Notifications', port: 50052, position: { x: 0, y: 0 }, rpcs: [], httpPort: null, routes: [], tables: [], consumers: [], jobs: [] })
    connectConsumer(p, 'Notifications', 'orders.placed')
    expect(consumersOf(p, 'orders.placed').map((s) => s.name)).toEqual(['Orders', 'Notifications'])
  })
})

describe('connecting a topic to a service', () => {
  it('creates a consumer named after the topic', () => {
    const p = project()
    p.services.push({ name: 'Notifications', port: 50052, position: { x: 0, y: 0 }, rpcs: [], httpPort: null, routes: [], tables: [], consumers: [], jobs: [] })
    expect(canConsume(p, 'Notifications', 'orders.placed')).toBe(true)
    expect(connectConsumer(p, 'Notifications', 'orders.placed')).toMatchObject({ name: 'on_orders_placed', topic: 'orders.placed', steps: [] })
    expect(p.services[1]!.consumers).toHaveLength(1)
  })
  it('refuses a topic the service already consumes, and unknown ends', () => {
    const p = project()
    p.services.push({ name: 'Notifications', port: 50052, position: { x: 0, y: 0 }, rpcs: [], httpPort: null, routes: [], tables: [], consumers: [], jobs: [] })
    connectConsumer(p, 'Notifications', 'orders.placed')
    expect(canConsume(p, 'Notifications', 'orders.placed')).toBe(false)
    expect(connectConsumer(p, 'Notifications', 'orders.placed')).toBeNull()
    expect(connectConsumer(p, 'Nope', 'orders.placed')).toBeNull()
    expect(connectConsumer(p, 'Notifications', 'nope')).toBeNull()
  })
})

describe('declaring producers', () => {
  const withNotifications = () => {
    const p = project()
    p.services.push({ name: 'Notifications', port: 50052, position: { x: 0, y: 0 }, rpcs: [], httpPort: null, routes: [], tables: [], consumers: [], jobs: [] })
    return p
  }
  it('makes a service a producer once, and only if both ends exist', () => {
    const p = withNotifications()
    expect(canProduce(p, 'Notifications', 'orders.placed')).toBe(true)
    expect(declareProducer(p, 'Notifications', 'orders.placed')).toBe(true)
    expect(p.topics[0]!.producers).toEqual(['Orders', 'Notifications'])
    expect(declareProducer(p, 'Notifications', 'orders.placed')).toBe(false)
    expect(declareProducer(p, 'Nope', 'orders.placed')).toBe(false)
    expect(declareProducer(p, 'Notifications', 'nope')).toBe(false)
    expect(p.topics[0]!.producers).toEqual(['Orders', 'Notifications'])
  })
  it('follows a renamed service and forgets a deleted one', () => {
    const p = withNotifications()
    declareProducer(p, 'Notifications', 'orders.placed')
    renameService(p, 'Notifications', 'Alerts')
    expect(p.topics[0]!.producers).toEqual(['Orders', 'Alerts'])
    removeService(p, 'Alerts')
    expect(p.topics[0]!.producers).toEqual(['Orders'])
    expect(p.services.map((s) => s.name)).toEqual(['Orders'])
  })
})

describe('schema kinds', () => {
  it('creates events for topics and proto schemas for RPCs', () => {
    const p = project()
    const rpc = addRpc(p, p.services[0]!)
    expect(p.messages.find((m) => m.name === rpc.request)!.kind).toBe('proto')
    expect(p.messages.find((m) => m.name === rpc.response)!.kind).toBe('proto')
    const topic = addTopic(p, { x: 0, y: 0 })
    expect(p.messages.find((m) => m.name === topic.message)!.kind).toBe('event')
  })
  it('makes an event when a topic is added to a project without one', () => {
    const p = project()
    p.messages = p.messages.filter((m) => m.kind !== 'event')
    p.topics = []
    const topic = addTopic(p, { x: 0, y: 0 })
    expect(p.messages.find((m) => m.name === topic.message)).toMatchObject({ kind: 'event' })
  })
  it('names new schemas after their kind', () => {
    const p = project()
    expect(addMessage(p, undefined, 'api')).toMatchObject({ name: 'NewApiSchema', kind: 'api' })
    expect(addMessage(p, undefined, 'event')).toMatchObject({ name: 'NewEvent', kind: 'event' })
  })
  it('keeps API schemas apart from protobuf ones when nesting', () => {
    const p = project()
    addMessage(p, 'Dto', 'api')
    expect(nestable(p, 'event').map((m) => m.name)).not.toContain('Dto')
    expect(nestable(p, 'proto').map((m) => m.name)).toContain('Placed')
    expect(nestable(p, 'api').map((m) => m.name)).toEqual(['Dto'])
  })
})

describe('HTTP routes', () => {
  it('reads the parameters of a path', () => {
    expect(pathParams('/orders/{order_id}/items/{item}')).toEqual(['order_id', 'item'])
    expect(pathParams('/orders')).toEqual([])
    expect(defaultStatus('post')).toBe(201)
    expect(defaultStatus('get')).toBe(200)
  })
  it('creates a route with its own API schemas and gives the service an HTTP port', () => {
    const p = project()
    const route = addRoute(p, p.services[0]!)
    expect(route).toMatchObject({ name: 'new_route', method: 'get', path: '/new-route', status: 200 })
    expect(p.messages.find((m) => m.name === route.request)!.kind).toBe('api')
    expect(p.messages.find((m) => m.name === route.response)!.kind).toBe('api')
    expect(p.services[0]!.httpPort).toBe(8081)
    // The port is kept, and the next route and schemas get fresh names.
    const next = addRoute(p, p.services[0]!)
    expect(next.name).toBe('new_route2')
    expect(next.request).not.toBe(route.request)
    expect(p.services[0]!.httpPort).toBe(8081)
  })
  it('takes the next free HTTP port for a second service', () => {
    const p = project()
    p.services.push({ name: 'Billing', port: 50052, position: { x: 0, y: 0 }, rpcs: [], httpPort: null, routes: [], tables: [], consumers: [], jobs: [] })
    addRoute(p, p.services[0]!)
    addRoute(p, p.services[1]!)
    expect(p.services.map((s) => s.httpPort)).toEqual([8081, 8082])
  })
  it('follows renames: schemas, and the `{param}` of the path when its field is renamed', () => {
    const p = project()
    const route = addRoute(p, p.services[0]!)
    route.path = '/orders/{id}'
    const request = p.messages.find((m) => m.name === route.request)!
    renameField(p, request.name, 'id', 'order_id')
    expect(route.path).toBe('/orders/{order_id}')
    renameMessage(p, route.response, 'OrderView')
    expect(route.response).toBe('OrderView')
  })
})

describe('endpoint configurator helpers', () => {
  const withRoute = () => {
    const p = project()
    const route = addRoute(p, p.services[0]!)
    const request = p.messages.find((m) => m.name === route.request)!
    request.fields.push({ name: 'tags', type: { kind: 'string' }, repeated: true, optional: false })
    request.fields.push({ name: 'limit', type: { kind: 'int32' }, repeated: false, optional: false })
    return { p, route, request }
  }
  it('writes an example JSON, leaving out the path parameters', () => {
    const { p, route } = withRoute()
    expect(JSON.parse(exampleJson(p, route.request))).toEqual({ id: 'string', tags: ['string'], limit: 0 })
    expect(JSON.parse(exampleJson(p, route.request, (f) => f.name !== 'id'))).toEqual({ tags: ['string'], limit: 0 })
  })
  it('nests schemas, and stops on a schema that contains itself', () => {
    const p = project()
    const node = addMessage(p, 'Node', 'api')
    node.fields.push({ name: 'child', type: { kind: 'message', name: 'Node' }, repeated: true, optional: false })
    const json = JSON.parse(exampleJson(p, 'Node'))
    expect(json.child[0].child[0].child[0].child[0]).toEqual({})
  })
  it('writes the query string of a GET', () => {
    const { p, route } = withRoute()
    expect(exampleQuery(p, route.request, (f) => f.name === 'limit')).toBe('?limit=0')
    expect(exampleQuery(p, route.request, () => false)).toBe('')
  })
  it('opens the endpoint of a diagnostic that points into a route', () => {
    const { p, route } = withRoute()
    expect(selectionForPath(p, '/services/0/routes/0/path')).toEqual({ kind: 'route', service: 'Orders', route: route.name })
    expect(selectionForPath(p, '/services/0/routes/0/steps/2')).toMatchObject({ kind: 'route' })
    expect(selectionForPath(p, '/services/0/rpcs/0/steps/1')).toEqual({ kind: 'service', name: 'Orders' })
    expect(selectionForPath(p, '/services/0/routes/7/path')).toEqual({ kind: 'service', name: 'Orders' })
  })
})

describe('tables and the SQL block', () => {
  const withOrders = () => {
    const p = project()
    p.messages.push({ name: 'GetOrderRequest', kind: 'api', fields: [{ name: 'order_id', type: { kind: 'string' }, repeated: false, optional: false }] })
    const svc = p.services[0]!
    const t = addTable(svc)
    t.name = 'orders'
    t.columns = [
      { name: 'order_id', type: 'string', nullable: false, primaryKey: true, default: null },
      { name: 'amount', type: 'int64', nullable: false, primaryKey: false, default: null },
      { name: 'note', type: 'string', nullable: true, primaryKey: false, default: null },
      { name: 'created_at', type: 'timestamp', nullable: false, primaryKey: false, default: 'now()' },
    ]
    return { p, svc }
  }

  it('creates a table with a uuid key, and nullable columns', () => {
    const p = project()
    const t = addTable(p.services[0]!)
    expect(t).toMatchObject({ name: 'new_table', columns: [{ name: 'id', type: 'uuid', primaryKey: true, nullable: false }] })
    expect(addTable(p.services[0]!).name).toBe('new_table2')
    expect(addColumn(t)).toMatchObject({ name: 'column', type: 'string', nullable: true })
  })

  it('names the row after the table and the step, like the Rust side', () => {
    expect(sqlRowName('orders', 'h6')).toBe('OrdersRowH6')
    expect(sqlRowName('order_items', 'a1b2')).toBe('OrderItemsRowA1b2')
  })

  it('types the variable of a select after the ticked columns', () => {
    const { p } = withOrders()
    const step = makeStep('sql', p, inputScope('GetOrderRequest'), { service: 'Orders', response: null })
    if (step.type !== 'sql') throw new Error('not a sql step')
    step.columns = ['order_id', 'note']
    p.services[0]!.routes = [{ name: 'r', method: 'get', path: '/r', request: 'GetOrderRequest', response: 'GetOrderRequest', status: 200, steps: [step] }]
    const scope = scopeAfter(p, inputScope('GetOrderRequest'), step)
    const paths = candidates(p, scope).map((c) => [c.path, c.type.type.kind, c.type.optional])
    expect(paths).toContainEqual([`${step.bind}.order_id`, 'string', false])
    expect(paths).toContainEqual([`${step.bind}.note`, 'string', true]) // a nullable column is optional
    expect(paths.map((x) => x[0])).not.toContain(`${step.bind}.amount`) // not ticked: not there
  })

  it('reads timestamps as text', () => {
    const { p } = withOrders()
    const step = makeStep('sql', p, inputScope('GetOrderRequest'), { service: 'Orders', response: null })
    if (step.type !== 'sql') throw new Error('not a sql step')
    p.services[0]!.routes = [{ name: 'r', method: 'get', path: '/r', request: 'GetOrderRequest', response: 'GetOrderRequest', status: 200, steps: [step] }]
    const cand = candidates(p, scopeAfter(p, inputScope('GetOrderRequest'), step)).find((c) => c.path.endsWith('.created_at'))
    expect(cand?.type.type.kind).toBe('string')
  })

  it('starts from the row the request already names: the key column is matched to `input.order_id`', () => {
    const { p } = withOrders()
    const scope = inputScope('GetOrderRequest') // has `order_id`
    const step = makeStep('sql', p, scope, { service: 'Orders', response: null })
    expect(step).toMatchObject({
      op: 'select',
      table: 'orders',
      columns: ['order_id', 'amount', 'note', 'created_at'],
      where: [{ column: 'order_id', op: 'eq', value: { kind: 'path', path: 'input.order_id' } }],
    })
  })

  it('follows a renamed table and a renamed column in the SQL steps', () => {
    const { p, svc } = withOrders()
    const step = makeStep('sql', p, inputScope('GetOrderRequest'), { service: 'Orders', response: null })
    if (step.type !== 'sql') throw new Error('not a sql step')
    step.orderBy = 'amount'
    step.values = [{ field: 'amount', value: { kind: 'literal', value: 1 } }]
    svc.routes = [{ name: 'r', method: 'get', path: '/r', request: 'GetOrderRequest', response: 'GetOrderRequest', status: 200, steps: [step] }]
    renameTable(p, 'Orders', 'orders', 'purchases')
    renameColumn(p, 'Orders', 'purchases', 'order_id', 'id')
    renameColumn(p, 'Orders', 'purchases', 'amount', 'total')
    expect(svc.tables[0]!.name).toBe('purchases')
    expect(svc.tables[0]!.columns.map((c) => c.name)).toEqual(['id', 'total', 'note', 'created_at'])
    expect(step.table).toBe('purchases')
    expect(step.columns).toEqual(['id', 'total', 'note', 'created_at'])
    expect(step.where[0]!.column).toBe('id')
    expect(step.orderBy).toBe('total')
    expect(step.values[0]!.field).toBe('total')
  })

  const withLines = () => {
    const { p, svc } = withOrders()
    const lines = addTable(svc)
    lines.name = 'order_lines'
    lines.columns = [
      { name: 'order_id', type: 'string', nullable: false, primaryKey: true, default: null },
      { name: 'line_no', type: 'int32', nullable: false, primaryKey: true, default: null },
    ]
    const shipments = addTable(svc)
    shipments.name = 'shipments'
    shipments.columns = [
      { name: 'id', type: 'uuid', nullable: false, primaryKey: true, default: 'gen_random_uuid()' },
      { name: 'order_ref', type: 'string', nullable: true, primaryKey: false, default: null },
      { name: 'line_no', type: 'int32', nullable: true, primaryKey: false, default: null },
    ]
    return { p, svc, lines, shipments }
  }

  it('starts a composite foreign key on the primary key of a table, with an unambiguous name', () => {
    const { svc, shipments } = withLines()
    const fk = addForeignKey(svc, shipments)!
    // The first table that has a primary key is `orders` (one column): the rest is edited by hand.
    expect(fk).toMatchObject({ refTable: 'orders', refColumns: ['order_id'], onDelete: 'noAction', onUpdate: 'noAction' })
    retargetForeignKey(svc, shipments, fk, 'order_lines')
    // Both key columns are paired, keeping the column already chosen for the first one.
    expect(fk.refColumns).toEqual(['order_id', 'line_no'])
    expect(fk.columns).toHaveLength(2)
    expect(addForeignKey(svc, shipments)!.name).not.toBe(fk.name)
  })

  it('pairs a foreign key with the column of the same name and type, and never with its own key', () => {
    const { svc, shipments } = withLines()
    shipments.columns.push({ name: 'order_id', type: 'string', nullable: true, primaryKey: false, default: null })
    expect(addForeignKey(svc, shipments)).toMatchObject({ columns: ['order_id'], refColumns: ['order_id'] })
    // A table pointing at itself: `order_id` is the referenced side, so the first other string column is the referencing one.
    const orders = svc.tables[0]!
    expect(addForeignKey(svc, orders)).toMatchObject({ columns: ['note'], refColumns: ['order_id'] })
  })

  it('refuses to start a foreign key when no table has a primary key', () => {
    const { svc, shipments } = withLines()
    for (const t of svc.tables) for (const c of t.columns) c.primaryKey = false
    expect(addForeignKey(svc, shipments)).toBeNull()
  })

  it('names the new constraints after the table and keeps the names apart', () => {
    const { shipments } = withLines()
    expect(addUnique(shipments)).toMatchObject({ name: 'shipments_id_uq', columns: ['id'] })
    expect(addIndex(shipments)).toMatchObject({ name: 'shipments_id_idx', columns: ['id'], unique: false })
    expect(addIndex(shipments).name).toBe('shipments_id_idx2')
  })

  it('follows a renamed table and a renamed column in the constraints, including those of other tables', () => {
    const { p, svc, lines, shipments } = withLines()
    shipments.uniques.push({ name: 'u', columns: ['order_ref', 'line_no'] })
    shipments.indexes.push({ name: 'i', columns: ['line_no'], unique: false })
    shipments.foreignKeys.push({ name: 'f', columns: ['order_ref', 'line_no'], refTable: 'order_lines', refColumns: ['order_id', 'line_no'], onDelete: 'cascade', onUpdate: 'noAction' })
    renameColumn(p, 'Orders', 'shipments', 'line_no', 'line')
    expect(shipments.uniques[0]!.columns).toEqual(['order_ref', 'line'])
    expect(shipments.indexes[0]!.columns).toEqual(['line'])
    expect(shipments.foreignKeys[0]!.columns).toEqual(['order_ref', 'line'])
    // The referenced side is untouched: it is another table's `line_no`.
    expect(shipments.foreignKeys[0]!.refColumns).toEqual(['order_id', 'line_no'])
    renameColumn(p, 'Orders', 'order_lines', 'line_no', 'position')
    expect(lines.columns.map((c) => c.name)).toEqual(['order_id', 'position'])
    expect(shipments.foreignKeys[0]!.refColumns).toEqual(['order_id', 'position'])
    renameTable(p, 'Orders', 'order_lines', 'lines')
    expect(shipments.foreignKeys[0]!.refTable).toBe('lines')
    expect(svc.tables.find((t) => t.name === 'lines')).toBe(lines)
  })

  it('types a jsonb column as text for the flows, like timestamps and uuids', () => {
    expect(columnType({ name: 'meta', type: 'jsonb', nullable: true, primaryKey: false, default: null })).toEqual({ type: { kind: 'string' }, repeated: false, optional: true })
  })

  it('shows the constraints in the SQL of the table', () => {
    const { shipments } = withLines()
    shipments.uniques.push({ name: 'shipments_uq', columns: ['order_ref', 'line_no'] })
    shipments.indexes.push({ name: 'shipments_idx', columns: ['order_ref'], unique: true })
    shipments.foreignKeys.push({ name: 'shipments_fk', columns: ['order_ref', 'line_no'], refTable: 'order_lines', refColumns: ['order_id', 'line_no'], onDelete: 'cascade', onUpdate: 'setNull' })
    expect(createTableSql(shipments).split('\n').slice(-3)).toEqual([
      'ALTER TABLE shipments ADD CONSTRAINT shipments_uq UNIQUE (order_ref, line_no);',
      'CREATE UNIQUE INDEX IF NOT EXISTS shipments_idx ON shipments (order_ref);',
      'ALTER TABLE shipments ADD CONSTRAINT shipments_fk FOREIGN KEY (order_ref, line_no) REFERENCES order_lines (order_id, line_no) ON DELETE CASCADE ON UPDATE SET NULL;',
    ])
  })
})

describe('uuid, optional and list variables', () => {
  const text = (kind: 'string' | 'uuid', extra: Partial<{ repeated: boolean; optional: boolean }> = {}) => ({ type: { kind }, repeated: false, optional: false, ...extra })

  it('lets uuid and string fill each other, like the validator', () => {
    expect(fits(text('uuid'), false, text('string'))).toBe(true)
    expect(fits(text('string'), false, text('uuid'))).toBe(true)
    expect(fits(text('uuid'), false, text('uuid', { optional: true }))).toBe(true)
    // ...but a list of them is not one of them, and a number is neither.
    expect(fits(text('uuid', { repeated: true }), false, text('string'))).toBe(false)
    expect(fits({ type: { kind: 'int64' }, repeated: false, optional: false }, false, text('uuid'))).toBe(false)
  })

  it('reads a uuid column as a uuid, and the other text columns as text', () => {
    expect(columnFieldType('uuid')).toEqual({ kind: 'uuid' })
    expect(columnFieldType('timestamp')).toEqual({ kind: 'string' })
    expect(columnFieldType('jsonb')).toEqual({ kind: 'string' })
  })

  it('binds a variable with the shape it was declared with', () => {
    const p = project()
    const let_ = (optional: boolean, repeated: boolean) =>
      ({ type: 'let', id: '1', name: 'x', valueType: { kind: 'uuid' }, optional, repeated, value: { kind: 'literal', value: null } }) as const
    expect(scopeAfter(p, inputScope('Order'), let_(true, false)).x).toEqual({ type: { kind: 'uuid' }, repeated: false, optional: true })
    expect(scopeAfter(p, inputScope('Order'), let_(false, true)).x).toEqual({ type: { kind: 'uuid' }, repeated: true, optional: false })
    // A list is never optional, whatever the file says.
    expect(scopeAfter(p, inputScope('Order'), let_(true, true)).x).toMatchObject({ repeated: true, optional: false })
  })

  it('shows uuid fields as a uuid in the JSON example', () => {
    const p = project()
    p.messages.push({ name: 'Ref', kind: 'api', fields: [{ name: 'id', type: { kind: 'uuid' }, repeated: false, optional: false }] })
    expect(exampleJson(p, 'Ref')).toContain('00000000-0000-0000-0000-000000000000')
  })
})

describe('password and token steps', () => {
  it('bind a string, and a bool for the verification, for the steps that follow', () => {
    const p = project()
    const scopes = scopesOf(p, inputScope('Order'), [
      { type: 'randomToken', id: '1', bind: 'token' },
      { type: 'sha256', id: '2', input: { kind: 'path', path: 'token' }, bind: 'digest' },
      { type: 'hashPassword', id: '3', input: { kind: 'path', path: 'digest' }, bind: 'hash' },
      { type: 'verifyPassword', id: '4', password: { kind: 'path', path: 'token' }, stored: { kind: 'path', path: 'hash' }, bind: 'ok' },
      { type: 'log', id: '5', level: 'info', message: '' },
    ])
    expect(Object.keys(scopes[4]!)).toEqual(['input', 'token', 'digest', 'hash', 'ok'])
    expect(scopes[4]!.token).toEqual(scalar('string'))
    expect(scopes[4]!.ok).toEqual(scalar('bool'))
  })

  it('start from the password the request carries, and from no stored hash', () => {
    const p = project()
    p.messages.push({ name: 'Login', kind: 'api', fields: [{ name: 'email', type: { kind: 'string' }, repeated: false, optional: false }, { name: 'password', type: { kind: 'string' }, repeated: false, optional: false }] })
    const ctx = { service: 'Orders', response: null }
    expect(makeStep('hashPassword', p, inputScope('Login'), ctx)).toMatchObject({ input: { kind: 'path', path: 'input.password' }, bind: 'password_hash' })
    expect(makeStep('verifyPassword', p, inputScope('Login'), ctx)).toMatchObject({
      password: { kind: 'path', path: 'input.password' },
      stored: { kind: 'literal', value: null },
      bind: 'password_ok',
    })
    // A hash already in scope is what a verification checks against, and never what it checks.
    const withHash = { ...inputScope('Login'), password_hash: scalar('string') }
    expect(makeStep('verifyPassword', p, withHash, ctx)).toMatchObject({
      password: { kind: 'path', path: 'input.password' },
      stored: { kind: 'path', path: 'password_hash' },
    })
    // Names stay free of those already in scope.
    const scope = { ...inputScope('Login'), token: scalar('string') }
    expect(makeStep('randomToken', p, scope, ctx)).toMatchObject({ bind: 'token2' })
  })
})

describe('rate limit and the response of a route', () => {
  const ctx = { service: 'Orders', response: 'Placed' }

  it('counts by e-mail when the request has one, else for everyone', () => {
    const p = project()
    p.messages.push({ name: 'Login', kind: 'api', fields: [{ name: 'email', type: { kind: 'string' }, repeated: false, optional: false }] })
    expect(makeStep('rateLimit', p, inputScope('Login'), ctx)).toMatchObject({ key: { kind: 'path', path: 'input.email' }, max: 5, windowSecs: 900 })
    expect(makeStep('rateLimit', p, inputScope('Order'), ctx)).toMatchObject({ key: { kind: 'literal', value: 'global' } })
  })

  it('starts a response on the route status and with no cookie', () => {
    expect(makeStep('respond', project(), inputScope('Order'), ctx)).toMatchObject({ status: null, cookies: [] })
  })
})

describe('SQL steps that give rows back', () => {
  type SqlStep = Extract<import('../types').Step, { type: 'sql' }>
  const update = (over: Partial<SqlStep> = {}): SqlStep =>
    ({
      type: 'sql',
      id: 'u1',
      op: 'update',
      table: 'orders',
      bind: 'consumed',
      columns: ['order_id', 'note'],
      many: false,
      into: null,
      where: [{ column: 'order_id', op: 'eq', value: { kind: 'path', path: 'input.id' } }, { column: 'created_at', op: 'gt', value: { kind: 'now' } }],
      values: [{ field: 'amount', value: { kind: 'literal', value: 1 } }],
      orderBy: null,
      descending: false,
      limit: null,
      missing: null,
      ...over,
    }) as SqlStep

  it('binds the returned row, typed after the ticked columns, like a select', () => {
    const p = project()
    const { svc } = (() => {
      const svc = p.services[0]!
      const t = addTable(svc)
      t.name = 'orders'
      t.columns = [
        { name: 'order_id', type: 'string', nullable: false, primaryKey: true, default: null },
        { name: 'amount', type: 'int64', nullable: false, primaryKey: false, default: null },
        { name: 'note', type: 'string', nullable: true, primaryKey: false, default: null },
        { name: 'created_at', type: 'timestamp', nullable: false, primaryKey: false, default: 'now()' },
      ]
      svc.routes = [{ name: 'r', method: 'post', path: '/r', request: 'Order', response: 'Placed', status: 200, steps: [update()] }]
      return { svc }
    })()
    expect(scopeAfter(p, inputScope('Order'), update()).consumed).toMatchObject({ type: { kind: 'message' }, repeated: false })
    const row = sqlRows(p).get(sqlRowName('orders', 'u1'))!
    expect(row.fields).toEqual([
      { name: 'order_id', type: { kind: 'string' }, repeated: false, optional: false },
      { name: 'note', type: { kind: 'string' }, repeated: false, optional: true },
    ])
    // An update that gives nothing back binds nothing.
    expect(scopeAfter(p, inputScope('Order'), update({ columns: [], bind: null }))).toEqual(inputScope('Order'))
    expect(svc.routes).toHaveLength(1)
  })

  it('reads as the SQL it generates', () => {
    expect(sqlPreview(update())).toBe('UPDATE orders SET amount = 1 WHERE order_id = input.id AND created_at > now() RETURNING order_id, note')
    expect(sqlPreview(update({ columns: [], bind: null }))).not.toContain('RETURNING')
  })
})

describe('jobs', () => {
  it('start empty, hourly, and take a name of their own', () => {
    const svc = project().services[0]!
    expect(addJob(svc)).toEqual({ name: 'job', everySecs: 3600, steps: [] })
    expect(addJob(svc).name).toBe('job2')
  })
  it('say their period like a person', () => {
    expect([30, 60, 420, 3600, 5400, 86400, 172800].map(everyLabel)).toEqual(['30 s', '1 min', '7 min', '1 h', '90 min', '1 j', '2 j'])
  })
  it('are walked like the other handlers: renaming a message reaches the steps of a job', () => {
    const p = project()
    p.services[0]!.jobs.push({ name: 'j', everySecs: 60, steps: [{ type: 'publish', id: 'p', topic: 'orders.placed', payload: [{ field: 'id', value: { kind: 'path', path: 'x' } }] }] })
    renameField(p, 'Placed', 'id', 'ident')
    expect((p.services[0]!.jobs[0]!.steps[0] as Extract<import('../types').Step, { type: 'publish' }>).payload[0]!.field).toBe('ident')
  })
})

describe('idempotent consumers', () => {
  it('can be told apart by a single string, uuid or integer field of the event', () => {
    const p = project()
    p.messages.push({
      name: 'Rich',
      kind: 'event',
      fields: [
        { name: 'id', type: { kind: 'string' }, repeated: false, optional: false },
        { name: 'ref', type: { kind: 'uuid' }, repeated: false, optional: false },
        { name: 'n', type: { kind: 'int64' }, repeated: false, optional: false },
        { name: 'maybe', type: { kind: 'string' }, repeated: false, optional: true },
        { name: 'tags', type: { kind: 'string' }, repeated: true, optional: false },
        { name: 'ratio', type: { kind: 'double' }, repeated: false, optional: false },
        { name: 'customer', type: { kind: 'message', name: 'Customer' }, repeated: false, optional: false },
      ],
    })
    expect(eventKeys(p, 'Rich')).toEqual(['id', 'ref', 'n'])
    expect(eventKeys(p, 'Nope')).toEqual([])
  })

  it('follow a renamed field of the event', () => {
    const p = project()
    const consumer = p.services[0]!.consumers[0]!
    consumer.idempotentOn = 'id'
    renameField(p, 'Placed', 'id', 'event_id')
    expect(consumer.idempotentOn).toBe('event_id')
    // Another message with a field of the same name is not concerned.
    renameField(p, 'Order', 'id', 'order_id')
    expect(consumer.idempotentOn).toBe('event_id')
  })
})

describe('signed webhooks', () => {
  it('start on Stripe\'s header, a secret of the environment and its five minutes', () => {
    expect(makeStep('verifyHmac', project(), inputScope('Order'), { service: 'Orders', response: 'Placed' })).toMatchObject({
      header: 'Stripe-Signature',
      secretEnv: 'STRIPE_WEBHOOK_SECRET',
      toleranceSecs: 300,
    })
  })
})

describe('encryption steps', () => {
  const ctx = { service: 'Orders', response: null }
  it('seal the secret of the request, and open what was just sealed', () => {
    const p = project()
    p.messages.push({ name: 'Totp', kind: 'api', fields: [{ name: 'secret', type: { kind: 'string' }, repeated: false, optional: false }] })
    const seal = makeStep('encrypt', p, inputScope('Totp'), ctx)
    expect(seal).toMatchObject({ type: 'encrypt', keyEnv: 'ENCRYPTION_KEY', input: { kind: 'path', path: 'input.secret' }, bind: 'sealed' })
    const scope = scopeAfter(p, inputScope('Totp'), seal)
    expect(scope.sealed).toEqual(scalar('string'))
    expect(makeStep('decrypt', p, scope, ctx)).toMatchObject({ type: 'decrypt', input: { kind: 'path', path: 'sealed' }, bind: 'opened' })
  })
})

describe('JWT steps', () => {
  const ctx = { service: 'Orders', response: 'Placed', http: true }

  it('start on a signing key of the environment, the service as issuer and a `sub` claim', () => {
    const p = project()
    p.messages.push({ name: 'Login', kind: 'api', fields: [{ name: 'email', type: { kind: 'string' }, repeated: false, optional: false }] })
    expect(makeStep('signJwt', p, inputScope('Login'), ctx)).toMatchObject({
      keyEnv: 'JWT_SIGNING_KEY',
      issuer: 'orders',
      ttlSecs: 300,
      claims: [{ field: 'sub', value: { kind: 'path', path: 'input.email' } }],
      bind: 'access_token',
    })
  })

  it('read the claims as the API schema that looks like them, and bind it', () => {
    const p = project()
    p.messages.push({ name: 'Other', kind: 'api', fields: [] }, { name: 'AccessClaims', kind: 'api', fields: [{ name: 'sub', type: { kind: 'string' }, repeated: false, optional: false }] })
    const step = makeStep('verifyJwt', p, inputScope('Order'), ctx)
    expect(step).toMatchObject({ type: 'verifyJwt', schema: 'AccessClaims', source: { kind: 'keys', keyEnvs: ['JWT_SIGNING_KEY'] }, bind: 'claims' })
    expect(scopeAfter(p, inputScope('Order'), step).claims).toEqual({ type: { kind: 'message', name: 'AccessClaims' }, repeated: false, optional: false })
    // Renaming the schema is followed by the step.
    p.services[0]!.routes = [{ name: 'r', method: 'post', path: '/r', request: 'Other', response: 'Other', status: 200, steps: [step] }]
    renameMessage(p, 'AccessClaims', 'Claims')
    expect((p.services[0]!.routes[0]!.steps[0] as { schema: string }).schema).toBe('Claims')
  })

  it('publish the keys in the schemas of a JWKS, which are created once', () => {
    const p = project()
    ensureJwksSchemas(p)
    ensureJwksSchemas(p)
    expect(p.messages.filter((m) => m.name === 'Jwks' || m.name === 'Jwk')).toHaveLength(2)
    expect(p.messages.find((m) => m.name === 'Jwk')!.fields.map((f) => f.name)).toEqual(['kty', 'crv', 'alg', 'kid', 'x'])
    expect(p.messages.find((m) => m.name === 'Jwks')!.fields[0]).toMatchObject({ name: 'keys', repeated: true, type: { kind: 'message', name: 'Jwk' } })
    const step = makeStep('jwks', p, inputScope('Order'), ctx)
    expect(scopeAfter(p, inputScope('Order'), step).published).toMatchObject({ type: { kind: 'message', name: 'Jwks' } })
  })
})

describe('second factor steps', () => {
  const ctx = { service: 'Orders', response: 'Placed', http: true }
  const text = (name: string) => ({ name, type: { kind: 'string' as const }, repeated: false, optional: false })

  it('enrol on the account of the request, and bind the schema that holds the secret', () => {
    const p = project()
    p.messages.push({ name: 'Enrol', kind: 'api', fields: [text('email')] })
    ensureTotpSchema(p)
    ensureTotpSchema(p)
    expect(p.messages.filter((m) => m.name === 'TotpSetup')).toHaveLength(1)
    expect(p.messages.find((m) => m.name === 'TotpSetup')!.fields.map((f) => f.name)).toEqual(['sealed', 'otpauth_url', 'secret'])
    const step = makeStep('totpGenerate', p, inputScope('Enrol'), ctx)
    expect(step).toMatchObject({ keyEnv: 'TOTP_ENCRYPTION_KEY', issuer: 'Orders', account: { kind: 'path', path: 'input.email' }, bind: 'setup' })
    expect(scopeAfter(p, inputScope('Enrol'), step).setup).toEqual({ type: { kind: 'message', name: 'TotpSetup' }, repeated: false, optional: false })
  })

  it('verify the code of the request against a sealed secret', () => {
    const p = project()
    p.messages.push({ name: 'Check', kind: 'api', fields: [text('account'), text('code')] })
    expect(makeStep('totpVerify', p, inputScope('Check'), ctx)).toMatchObject({
      type: 'totpVerify',
      keyEnv: 'TOTP_ENCRYPTION_KEY',
      code: { kind: 'path', path: 'input.code' },
      account: { kind: 'path', path: 'input.account' },
    })
  })

  it('issue a list of texts, and spend the code of the request', () => {
    const p = project()
    p.messages.push({ name: 'Check', kind: 'api', fields: [text('account'), text('code')] })
    const issue = makeStep('recoveryCodes', p, inputScope('Check'), ctx)
    expect(issue).toMatchObject({ type: 'recoveryCodes', count: 8, bind: 'recovery_codes' })
    expect(scopeAfter(p, inputScope('Check'), issue).recovery_codes).toEqual({ type: { kind: 'string' }, repeated: true, optional: false })
    expect(makeStep('recoveryCodeUse', p, inputScope('Check'), ctx)).toMatchObject({ type: 'recoveryCodeUse', code: { kind: 'path', path: 'input.code' } })
  })
})

describe('passkey steps', () => {
  const ctx = { service: 'Orders', response: 'Placed', http: true }
  const text = (name: string) => ({ name, type: { kind: 'string' as const }, repeated: false, optional: false })

  it('start a registration on the verified account, into a schema created once', () => {
    const p = project()
    p.messages.push({ name: 'Claims', kind: 'api', fields: [text('sub')] }, { name: 'Enrol', kind: 'api', fields: [text('name')] })
    ensurePasskeySchema(p)
    ensurePasskeySchema(p)
    expect(p.messages.filter((m) => m.name === 'PasskeyChallenge')).toHaveLength(1)
    expect(p.messages.find((m) => m.name === 'PasskeyChallenge')!.fields.map((f) => f.name)).toEqual(['state_id', 'options'])
    const scope = { ...inputScope('Enrol'), claims: { type: { kind: 'message' as const, name: 'Claims' }, repeated: false, optional: false } }
    const step = makeStep('passkeyRegisterStart', p, scope, ctx)
    expect(step).toMatchObject({ account: { kind: 'path', path: 'claims.sub' }, name: { kind: 'path', path: 'input.name' }, bind: 'challenge' })
    expect(scopeAfter(p, scope, step).challenge).toEqual({ type: { kind: 'message', name: 'PasskeyChallenge' }, repeated: false, optional: false })
  })

  it('finish a login on the answer of the request, and bind the account', () => {
    const p = project()
    p.messages.push({ name: 'Answer', kind: 'api', fields: [text('state_id'), text('credential')] })
    const step = makeStep('passkeyLoginFinish', p, inputScope('Answer'), ctx)
    expect(step).toMatchObject({ stateId: { kind: 'path', path: 'input.state_id' }, credential: { kind: 'path', path: 'input.credential' }, bind: 'account' })
    expect(scopeAfter(p, inputScope('Answer'), step).account).toEqual(scalar('string'))
    expect(makeStep('passkeyRegisterFinish', p, inputScope('Answer'), ctx)).toMatchObject({ type: 'passkeyRegisterFinish', credential: { kind: 'path', path: 'input.credential' } })
  })
})

describe('object storage steps', () => {
  const ctx = { service: 'Orders', response: 'Placed', http: true }
  const text = (name: string) => ({ name, type: { kind: 'string' as const }, repeated: false, optional: false })

  it('recognise the image of the request, PNG and JPEG up to 2 MiB, and bind its type', () => {
    const p = project()
    p.messages.push({ name: 'Upload', kind: 'api', fields: [text('image')] })
    const step = makeStep('imageSniff', p, inputScope('Upload'), ctx)
    expect(step).toMatchObject({ input: { kind: 'path', path: 'input.image' }, allow: ['png', 'jpeg'], maxBytes: 2097152, bind: 'content_type' })
    expect(scopeAfter(p, inputScope('Upload'), step).content_type).toEqual(scalar('string'))
  })

  it('store under the account, with the type that was found', () => {
    const p = project()
    p.messages.push({ name: 'Upload', kind: 'api', fields: [text('image')] })
    const scope = { ...inputScope('Upload'), content_type: scalar('string'), claims: { type: { kind: 'message' as const, name: 'Claims' }, repeated: false, optional: false } }
    p.messages.push({ name: 'Claims', kind: 'api', fields: [text('sub')] })
    expect(makeStep('objectPut', p, scope, ctx)).toMatchObject({
      bucketEnv: 'AVATAR_BUCKET',
      key: { kind: 'path', path: 'claims.sub' },
      body: { kind: 'path', path: 'input.image' },
      contentType: { kind: 'path', path: 'content_type' },
    })
  })

  it('read an object that may be absent', () => {
    const p = project()
    p.messages.push({ name: 'Who', kind: 'api', fields: [text('account')] })
    const step = makeStep('objectGet', p, inputScope('Who'), ctx)
    expect(step).toMatchObject({ type: 'objectGet', key: { kind: 'path', path: 'input.account' }, bind: 'object' })
    expect(scopeAfter(p, inputScope('Who'), step).object).toEqual({ type: { kind: 'string' }, repeated: false, optional: true })
    expect(makeStep('objectDelete', p, inputScope('Who'), ctx)).toMatchObject({ type: 'objectDelete', bucketEnv: 'AVATAR_BUCKET' })
  })
})
