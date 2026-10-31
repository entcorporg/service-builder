import {
  applyNodeChanges,
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  type Connection,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from '@xyflow/react'
import { type DragEvent, useCallback, useEffect, useMemo, useState } from 'react'
import { addService, addTopic, canConsume, canProduce, connectConsumer, declareProducer, walkSteps } from '../model'
import { useStore } from '../store'
import type { Diagnostic, Project, Service, Topic } from '../types'
import { Icon } from './Icon'
import { Badge } from './ui'

export const DND_TYPE = 'application/x-sb-element'

interface ServiceData extends Record<string, unknown> {
  service: Service
  errors: number
  warnings: number
}
interface TopicData extends Record<string, unknown> {
  topic: Topic
  errors: number
  warnings: number
}

type SbNode = Node<ServiceData, 'service'> | Node<TopicData, 'topic'>

function ServiceNode({ data, selected }: NodeProps<Node<ServiceData, 'service'>>) {
  const { service } = data
  return (
    <div className={`node service${selected ? ' selected' : ''}`}>
      <Handle type="target" position={Position.Left} id="in" />
      <Handle type="target" position={Position.Top} id="rpc-in" />
      <header>
        <Icon name="service" size={15} />
        <strong>{service.name}</strong>
        <span className="node-port mono">{service.kind === 'mta' ? 'mta' : `:${service.port}`}</span>
        {data.errors > 0 && <Badge kind="error" count={data.errors} />}
        {data.errors === 0 && data.warnings > 0 && <Badge kind="warning" count={data.warnings} />}
      </header>
      <ul>
        {service.rpcs.map((r) => (
          <li key={r.name} className="rpc">
            <span className="tag-pill rpc">RPC</span>
            <span className="mono">{r.name}</span>
          </li>
        ))}
        {service.routes.map((r) => (
          <li
            key={r.name}
            className="route nodrag nopan"
            title="Configurer cet endpoint"
            onClick={(e) => {
              e.stopPropagation()
              useStore.getState().select({ kind: 'route', service: service.name, route: r.name })
            }}
          >
            <span className={`tag-pill http ${r.method}`}>{r.method.toUpperCase()}</span>
            <span className="mono route-path">{r.path}</span>
          </li>
        ))}
        {service.consumers.map((c) => (
          <li key={c.name} className="consumer">
            <span className="tag-pill consumer">{c.mail ? 'MAIL' : 'KAFKA'}</span>
            <span className="mono">{c.mail ? c.topic : c.name}</span>
          </li>
        ))}
        {service.jobs.map((j) => (
          <li key={j.name} className="consumer">
            <span className="tag-pill consumer">TÂCHE</span>
            <span className="mono">{j.name}</span>
          </li>
        ))}
        {service.rpcs.length === 0 && service.routes.length === 0 && service.consumers.length === 0 && service.jobs.length === 0 && <li className="none">vide</li>}
      </ul>
      <Handle type="source" position={Position.Right} id="out" />
      <Handle type="source" position={Position.Bottom} id="rpc-out" />
    </div>
  )
}

function TopicNode({ data, selected }: NodeProps<Node<TopicData, 'topic'>>) {
  const { topic } = data
  return (
    <div className={`node topic${selected ? ' selected' : ''}`}>
      <Handle type="target" position={Position.Left} id="in" />
      <header>
        <Icon name="topic" size={15} />
        <strong className="mono">{topic.name}</strong>
        {data.errors > 0 && <Badge kind="error" count={data.errors} />}
        {data.errors === 0 && data.warnings > 0 && <Badge kind="warning" count={data.warnings} />}
      </header>
      <div className="topic-meta">
        <span className="mono">{topic.message}</span>
        <small>
          {topic.partitions} partition{topic.partitions > 1 ? 's' : ''}
          {topic.keyField ? ` · clé ${topic.keyField}` : ''}
        </small>
      </div>
      <Handle type="source" position={Position.Right} id="out" />
    </div>
  )
}

const nodeTypes = { service: ServiceNode, topic: TopicNode }

function countFor(diagnostics: Diagnostic[], prefix: string) {
  const mine = diagnostics.filter((d) => d.path === prefix || d.path.startsWith(`${prefix}/`))
  return {
    errors: mine.filter((d) => d.severity === 'error').length,
    warnings: mine.filter((d) => d.severity === 'warning').length,
  }
}

/** The graph is derived from the project: edges are never stored, drawing one edits the project (see `onConnect`). */
export function buildGraph(project: Project, diagnostics: Diagnostic[], selected: string | null) {
  const nodes: SbNode[] = []
  const edges: Edge[] = []

  project.services.forEach((service, i) => {
    const id = `svc:${service.name}`
    nodes.push({
      id,
      type: 'service',
      position: service.position,
      selected: selected === id,
      data: { service, ...countFor(diagnostics, `/services/${i}`) },
    })

    // Declared producers and publishing steps both draw the arrow (a step without declaration is also an error).
    const published = new Set<string>(project.topics.filter((t) => t.producers.includes(service.name)).map((t) => t.name))
    const called = new Map<string, Set<string>>()
    const handlers = [...service.rpcs.map((r) => r.steps), ...service.routes.map((r) => r.steps), ...service.consumers.map((c) => c.steps), ...service.jobs.map((j) => j.steps)]
    for (const steps of handlers) {
      walkSteps(steps, (s) => {
        if (s.type === 'publish' && s.topic) published.add(s.topic)
        if (s.type === 'callRpc' && s.service) called.set(s.service, (called.get(s.service) ?? new Set()).add(s.method))
      })
    }
    for (const topic of published) {
      if (!project.topics.some((t) => t.name === topic)) continue
      edges.push({
        id: `pub:${service.name}:${topic}`,
        source: id,
        sourceHandle: 'out',
        target: `top:${topic}`,
        targetHandle: 'in',
        className: 'edge-publish',
        markerEnd: { type: MarkerType.ArrowClosed },
      })
    }
    for (const c of service.consumers) {
      if (!project.topics.some((t) => t.name === c.topic)) continue
      edges.push({
        id: `sub:${service.name}:${c.name}`,
        source: `top:${c.topic}`,
        sourceHandle: 'out',
        target: id,
        targetHandle: 'in',
        className: 'edge-consume',
        markerEnd: { type: MarkerType.ArrowClosed },
      })
    }
    for (const [target, methods] of called) {
      if (!project.services.some((s) => s.name === target)) continue
      edges.push({
        id: `rpc:${service.name}:${target}`,
        source: id,
        sourceHandle: 'rpc-out',
        target: `svc:${target}`,
        targetHandle: 'rpc-in',
        className: 'edge-rpc',
        markerEnd: { type: MarkerType.ArrowClosed },
        label: [...methods].join(', '),
      })
    }
  })

  project.topics.forEach((topic, i) => {
    const id = `top:${topic.name}`
    nodes.push({
      id,
      type: 'topic',
      position: topic.position,
      selected: selected === id,
      data: { topic, ...countFor(diagnostics, `/topics/${i}`) },
    })
  })
  return { nodes, edges }
}

type Link = { kind: 'consume' | 'produce'; topic: string; service: string }

// Two arrows are drawable, both declare something: topic → service creates a consumer, service → topic
// makes the service a producer. RPC calls come from steps, the publishing itself from a Publish step.
function parse(c: { source: string; sourceHandle?: string | null; target: string; targetHandle?: string | null }): Link | null {
  if (c.source.startsWith('top:') && c.sourceHandle === 'out' && c.target.startsWith('svc:') && c.targetHandle === 'in')
    return { kind: 'consume', topic: c.source.slice('top:'.length), service: c.target.slice('svc:'.length) }
  if (c.source.startsWith('svc:') && c.sourceHandle === 'out' && c.target.startsWith('top:') && c.targetHandle === 'in')
    return { kind: 'produce', topic: c.target.slice('top:'.length), service: c.source.slice('svc:'.length) }
  return null
}

function CanvasInner() {
  const project = useStore((s) => s.project)
  const diagnostics = useStore((s) => s.analysis.diagnostics)
  const selection = useStore((s) => s.selection)
  const edit = useStore((s) => s.edit)
  const select = useStore((s) => s.select)
  const { screenToFlowPosition } = useReactFlow()

  const selectedId = selection?.kind === 'service' ? `svc:${selection.name}` : selection?.kind === 'topic' ? `top:${selection.name}` : null
  const graph = useMemo(() => buildGraph(project, diagnostics, selectedId), [project, diagnostics, selectedId])
  const [nodes, setNodes] = useState<SbNode[]>(graph.nodes)
  // Re-derive from the project but keep the sizes xyflow measured, or the minimap/fitView lose them.
  useEffect(
    () =>
      setNodes((prev) =>
        graph.nodes.map((n) => {
          const measured = prev.find((p) => p.id === n.id)?.measured
          return measured ? { ...n, measured } : n
        }),
      ),
    [graph.nodes],
  )

  const onNodesChange = useCallback((changes: NodeChange<SbNode>[]) => {
    // Selection is owned by the store; moves and measured sizes are applied locally.
    setNodes((ns) => applyNodeChanges(changes.filter((c) => c.type === 'position' || c.type === 'dimensions'), ns))
  }, [])

  const isValidConnection = useCallback(
    (c: Connection | Edge) => {
      const link = parse(c)
      if (!link) return false
      return link.kind === 'consume' ? canConsume(project, link.service, link.topic) : canProduce(project, link.service, link.topic)
    },
    [project],
  )
  const onConnect = (c: Connection) => {
    const link = parse(c)
    if (!link) return
    let done = false
    edit((p) => {
      done = link.kind === 'consume' ? !!connectConsumer(p, link.service, link.topic) : declareProducer(p, link.service, link.topic)
    })
    if (done) select({ kind: 'service', name: link.service })
  }

  const onDragOver = (e: DragEvent) => {
    if (e.dataTransfer.types.includes(DND_TYPE)) {
      e.preventDefault()
      e.dataTransfer.dropEffect = 'copy'
    }
  }

  const onDrop = (e: DragEvent) => {
    const kind = e.dataTransfer.getData(DND_TYPE)
    if (kind !== 'service' && kind !== 'topic') return
    e.preventDefault()
    const position = screenToFlowPosition({ x: e.clientX, y: e.clientY })
    let name = ''
    edit((p) => {
      name = kind === 'service' ? addService(p, position).name : addTopic(p, position).name
    })
    select({ kind, name })
  }

  return (
    <div className="canvas" onDragOver={onDragOver} onDrop={onDrop}>
      <ReactFlow
        nodes={nodes}
        edges={graph.edges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onNodeClick={(_, n) =>
          select(n.type === 'service' ? { kind: 'service', name: (n.data as ServiceData).service.name } : { kind: 'topic', name: (n.data as TopicData).topic.name })
        }
        onPaneClick={() => select(null)}
        onNodeDragStop={(_, n) =>
          edit((p) => {
            const target = n.type === 'service' ? p.services.find((s) => `svc:${s.name}` === n.id) : p.topics.find((t) => `top:${t.name}` === n.id)
            if (target) target.position = { x: Math.round(n.position.x), y: Math.round(n.position.y) }
          })
        }
        onConnect={onConnect}
        isValidConnection={isValidConnection}
        deleteKeyCode={null}
        fitView
        fitViewOptions={{ padding: 0.1, maxZoom: 1 }}
        minZoom={0.3}
        colorMode="dark"
        proOptions={{ hideAttribution: true }}
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1.4} />
        <Controls showInteractive={false} />
        <MiniMap pannable zoomable nodeColor={(n) => (n.type === 'service' ? '#6c7aff' : '#ffb04a')} maskColor="rgba(10,12,20,.7)" />
      </ReactFlow>
      {project.services.length === 0 && project.topics.length === 0 && (
        <div className="canvas-empty">
          <Icon name="graph" size={34} />
          <p>Glissez un <strong>Service</strong> ou un <strong>Topic</strong> ici pour commencer.</p>
        </div>
      )}
      <Legend />
    </div>
  )
}

function Legend() {
  return (
    <div className="legend">
      <span><i className="swatch publish" /> publie</span>
      <span><i className="swatch consume" /> consomme</span>
      <span><i className="swatch rpc" /> appel gRPC</span>
    </div>
  )
}

export function Canvas() {
  return (
    <ReactFlowProvider>
      <CanvasInner />
    </ReactFlowProvider>
  )
}
