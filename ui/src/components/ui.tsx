import { type ReactNode, useEffect, useRef, useState } from 'react'
import { Icon, type IconName } from './Icon'

/** Text input that applies its value on Enter/blur (renames cascade through the project, so no per-key edits). */
export function CommitInput(props: {
  value: string
  onCommit: (value: string) => void
  mono?: boolean
  placeholder?: string
  label?: string
}) {
  const [draft, setDraft] = useState(props.value)
  const cancelled = useRef(false)
  useEffect(() => setDraft(props.value), [props.value])
  const commit = () => {
    const v = draft.trim()
    if (!cancelled.current && v !== '' && v !== props.value) props.onCommit(v)
    else setDraft(props.value)
    cancelled.current = false
  }
  return (
    <input
      className={props.mono ? 'mono' : undefined}
      value={draft}
      placeholder={props.placeholder}
      aria-label={props.label}
      spellCheck={false}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === 'Enter') e.currentTarget.blur()
        if (e.key === 'Escape') {
          cancelled.current = true
          e.currentTarget.blur()
        }
      }}
    />
  )
}

export function NumberInput(props: { value: number; onChange: (v: number) => void; min?: number; step?: number; label?: string }) {
  const [text, setText] = useState(String(props.value))
  useEffect(() => setText(String(props.value)), [props.value])
  return (
    <input
      type="number"
      inputMode="numeric"
      value={text}
      min={props.min}
      step={props.step}
      aria-label={props.label}
      onChange={(e) => {
        setText(e.target.value)
        const n = Number(e.target.value)
        if (e.target.value !== '' && Number.isFinite(n)) props.onChange(n)
      }}
      onBlur={() => setText(String(props.value))}
    />
  )
}

export function Row(props: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="row">
      <span className="row-label">{props.label}</span>
      <span className="row-control">{props.children}</span>
      {props.hint && <span className="row-hint">{props.hint}</span>}
    </label>
  )
}

export function Section(props: { title: string; icon?: IconName; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="section">
      <header className="section-head">
        {props.icon && <Icon name={props.icon} size={14} />}
        <h3>{props.title}</h3>
        <span className="spacer" />
        {props.action}
      </header>
      {props.children}
    </section>
  )
}

export function IconButton(props: {
  icon: IconName
  title: string
  onClick: () => void
  disabled?: boolean
  danger?: boolean
  size?: number
}) {
  return (
    <button
      type="button"
      className={`icon-btn${props.danger ? ' danger' : ''}`}
      title={props.title}
      aria-label={props.title}
      disabled={props.disabled}
      onClick={props.onClick}
    >
      <Icon name={props.icon} size={props.size ?? 15} />
    </button>
  )
}

export function Collapsible(props: {
  title: ReactNode
  badge?: ReactNode
  actions?: ReactNode
  defaultOpen?: boolean
  children: ReactNode
  tone?: 'rpc' | 'consumer' | 'step'
}) {
  const [open, setOpen] = useState(props.defaultOpen ?? true)
  return (
    <div className={`card ${props.tone ?? ''}`}>
      <div className="card-head">
        <button type="button" className="card-toggle" onClick={() => setOpen(!open)} aria-expanded={open}>
          <span className={`chev${open ? ' open' : ''}`}>
            <Icon name="right" size={14} />
          </span>
          <span className="card-title">{props.title}</span>
        </button>
        {props.badge}
        {props.actions}
      </div>
      {open && <div className="card-body">{props.children}</div>}
    </div>
  )
}

export function Badge(props: { kind: 'error' | 'warning'; count?: number; title?: string }) {
  return (
    <span className={`badge ${props.kind}`} title={props.title}>
      <Icon name={props.kind === 'error' ? 'error' : 'warn'} size={12} />
      {props.count !== undefined && props.count > 1 ? props.count : null}
    </span>
  )
}

export function Empty(props: { children: ReactNode }) {
  return <p className="empty">{props.children}</p>
}
