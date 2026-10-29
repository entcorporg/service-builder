import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from './Icon'

interface TreeNode {
  name: string
  path: string
  /** Only files carry no children. */
  children: TreeNode[] | null
}

function buildTree(paths: string[]): TreeNode[] {
  const root: TreeNode[] = []
  for (const path of paths) {
    const parts = path.split('/')
    let level = root
    parts.forEach((name, i) => {
      const full = parts.slice(0, i + 1).join('/')
      if (i === parts.length - 1) return void level.push({ name, path: full, children: null })
      let dir = level.find((n) => n.children && n.path === full)
      if (!dir) level.push((dir = { name, path: full, children: [] }))
      level = dir.children!
    })
  }
  const sort = (nodes: TreeNode[]) => {
    // Folders first, then alphabetical, like every file explorer.
    nodes.sort((a, b) => Number(!a.children) - Number(!b.children) || a.name.localeCompare(b.name))
    for (const n of nodes) if (n.children) sort(n.children)
  }
  sort(root)
  return root
}

function ancestors(path: string): string[] {
  const parts = path.split('/').slice(0, -1)
  return parts.map((_, i) => parts.slice(0, i + 1).join('/'))
}

const extOf = (name: string) => (name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : '')

export function FileTree({ paths, active, onSelect }: { paths: string[]; active: string | null; onSelect: (path: string) => void }) {
  const tree = useMemo(() => buildTree(paths), [paths])
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const ref = useRef<HTMLDivElement>(null)

  // Whatever file becomes active is made visible.
  useEffect(() => {
    if (!active) return
    setCollapsed((prev) => {
      const open = ancestors(active)
      return open.some((d) => prev.has(d)) ? new Set([...prev].filter((d) => !open.includes(d))) : prev
    })
  }, [active])

  const toggle = (path: string, collapse?: boolean) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (collapse ?? !next.has(path)) next.add(path)
      else next.delete(path)
      return next
    })

  const onKeyDown = (e: KeyboardEvent) => {
    const rows = [...(ref.current?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? [])]
    const i = rows.indexOf(document.activeElement as HTMLElement)
    if (i < 0) return
    const row = rows[i]!
    const isDir = row.hasAttribute('aria-expanded')
    const open = row.getAttribute('aria-expanded') === 'true'
    const path = row.dataset.path!
    let handled = true
    if (e.key === 'ArrowDown') rows[Math.min(i + 1, rows.length - 1)]!.focus()
    else if (e.key === 'ArrowUp') rows[Math.max(i - 1, 0)]!.focus()
    else if (e.key === 'Home') rows[0]!.focus()
    else if (e.key === 'End') rows.at(-1)!.focus()
    else if (e.key === 'ArrowRight') {
      if (isDir && !open) toggle(path, false)
      else if (isDir) rows[i + 1]?.focus()
    } else if (e.key === 'ArrowLeft') {
      if (isDir && open) toggle(path, true)
      else rows.find((r) => r.dataset.path === path.slice(0, path.lastIndexOf('/')))?.focus()
    } else handled = false
    if (handled) e.preventDefault()
  }

  // One tab stop: the active file, or else the first row.
  const stop = active && paths.includes(active) ? active : (tree[0]?.path ?? null)

  const render = (nodes: TreeNode[], depth: number) =>
    nodes.map((n) => {
      const style = { paddingLeft: 8 + depth * 14 }
      if (!n.children) {
        return (
          <li key={n.path} role="none">
            <button
              type="button"
              role="treeitem"
              aria-level={depth + 1}
              aria-selected={n.path === active}
              data-path={n.path}
              data-ext={extOf(n.name)}
              tabIndex={n.path === stop ? 0 : -1}
              className={`tree-row leaf${n.path === active ? ' active' : ''}`}
              style={style}
              title={n.path}
              onClick={() => onSelect(n.path)}
            >
              <span className="tree-chevron" />
              <Icon name="file" size={14} />
              <span className="mono">{n.name}</span>
            </button>
          </li>
        )
      }
      const open = !collapsed.has(n.path)
      return (
        <li key={n.path} role="none">
          <button
            type="button"
            role="treeitem"
            aria-level={depth + 1}
            aria-expanded={open}
            data-path={n.path}
            tabIndex={n.path === stop ? 0 : -1}
            className="tree-row dir"
            style={style}
            title={n.path}
            onClick={() => toggle(n.path)}
          >
            <span className="tree-chevron">
              <Icon name={open ? 'down' : 'right'} size={12} />
            </span>
            <Icon name={open ? 'folderOpen' : 'folder'} size={14} />
            <span className="mono">{n.name}</span>
          </button>
          {open && <ul role="group">{render(n.children, depth + 1)}</ul>}
        </li>
      )
    })

  return (
    <div ref={ref} className="file-tree" onKeyDown={onKeyDown}>
      <ul role="tree" aria-label="Fichiers générés">
        {render(tree, 0)}
      </ul>
    </div>
  )
}
