import { describe, expect, it } from 'vitest'
import { autoMap, renameField, renameMessage, renameTopic, servicesUsingTopic } from '../model'
import { candidates, fits, inputScope, scalar, scopesOf, typeOfField } from '../scope'
import type { Project } from '../types'

const project = (): Project => ({
  schemaVersion: 1,
  name: 'T',
  broker: { bootstrapServers: 'localhost:9092' },
  messages: [
    { name: 'Customer', fields: [{ name: 'name', type: { kind: 'string' }, repeated: false, optional: false }] },
    {
      name: 'Order',
      fields: [
        { name: 'id', type: { kind: 'string' }, repeated: false, optional: false },
        { name: 'customer', type: { kind: 'message', name: 'Customer' }, repeated: false, optional: false },
        { name: 'note', type: { kind: 'string' }, repeated: false, optional: true },
      ],
    },
    { name: 'Placed', fields: [{ name: 'id', type: { kind: 'string' }, repeated: false, optional: false }] },
  ],
  topics: [{ name: 'orders.placed', message: 'Placed', partitions: 1, replicationFactor: 1, keyField: 'id', position: { x: 0, y: 0 } }],
  services: [
    {
      name: 'Orders',
      port: 50051,
      position: { x: 0, y: 0 },
      rpcs: [
        {
          name: 'Create',
          request: 'Order',
          response: 'Placed',
          steps: [
            { type: 'publish', id: 'a', topic: 'orders.placed', payload: [{ field: 'id', value: { kind: 'path', path: 'input.id' } }] },
            { type: 'respond', id: 'b', fields: [{ field: 'id', value: { kind: 'path', path: 'input.id' } }] },
          ],
        },
      ],
      consumers: [{ name: 'on_placed', topic: 'orders.placed', groupId: null, steps: [] }],
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
      { type: 'let', id: '1', name: 'total', valueType: { kind: 'int64' }, value: { kind: 'literal', value: 1 } },
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
  it('finds the services using a topic', () => {
    expect(servicesUsingTopic(project(), 'orders.placed').map((s) => s.name)).toEqual(['Orders'])
    expect(servicesUsingTopic(project(), 'nope')).toEqual([])
  })
})
