import hljs from 'highlight.js/lib/core'
import ini from 'highlight.js/lib/languages/ini'
import markdown from 'highlight.js/lib/languages/markdown'
import protobuf from 'highlight.js/lib/languages/protobuf'
import rust from 'highlight.js/lib/languages/rust'
import yaml from 'highlight.js/lib/languages/yaml'
import { useMemo } from 'react'
import { useStore } from '../store'
import { Icon } from './Icon'

hljs.registerLanguage('rust', rust)
hljs.registerLanguage('protobuf', protobuf)
hljs.registerLanguage('ini', ini)
hljs.registerLanguage('yaml', yaml)
hljs.registerLanguage('markdown', markdown)

function languageOf(path: string): string | null {
  if (path.endsWith('.rs')) return 'rust'
  if (path.endsWith('.proto')) return 'protobuf'
  if (path.endsWith('.toml')) return 'ini'
  if (path.endsWith('.yml') || path.endsWith('.yaml')) return 'yaml'
  if (path.endsWith('.md')) return 'markdown'
  return null
}

export function CodeView() {
  const files = useStore((s) => s.files)
  const stale = useStore((s) => s.filesStale)
  const active = useStore((s) => s.activeFile)
  const setActive = useStore((s) => s.setActiveFile)
  const file = files.find((f) => f.path === active) ?? files[0]

  const html = useMemo(() => {
    if (!file) return ''
    const lang = languageOf(file.path)
    return lang ? hljs.highlight(file.content, { language: lang }).value : escapeHtml(file.content)
  }, [file])

  // Group by directory for a readable tree.
  const groups = useMemo(() => {
    const m = new Map<string, string[]>()
    for (const f of files) {
      const i = f.path.lastIndexOf('/')
      const dir = i < 0 ? '.' : f.path.slice(0, i)
      m.set(dir, [...(m.get(dir) ?? []), f.path])
    }
    return [...m.entries()].sort(([a], [b]) => (a === '.' ? -1 : b === '.' ? 1 : a.localeCompare(b)))
  }, [files])

  if (files.length === 0) {
    return (
      <div className="code-empty">
        <Icon name="code" size={32} />
        <p>Le code sera généré dès que le projet n’aura plus d’erreurs.</p>
      </div>
    )
  }

  return (
    <div className="code-view">
      <nav className="code-tree" aria-label="Fichiers générés">
        {groups.map(([dir, paths]) => (
          <div key={dir}>
            <h4 className="mono">{dir}</h4>
            {paths.map((p) => (
              <button key={p} type="button" className={p === file?.path ? 'active' : ''} onClick={() => setActive(p)}>
                <Icon name="file" size={13} /> <span className="mono">{p.slice(p.lastIndexOf('/') + 1)}</span>
              </button>
            ))}
          </div>
        ))}
      </nav>
      <div className="code-main">
        <header>
          <span className="mono">{file?.path}</span>
          {stale && <span className="stale">Dernière version valide : le projet contient des erreurs</span>}
          <span className="spacer" />
          <button type="button" className="ghost" onClick={() => file && void navigator.clipboard?.writeText(file.content)}>
            <Icon name="copy" size={14} /> Copier
          </button>
        </header>
        <pre className="hljs" aria-label={`Contenu de ${file?.path}`}>
          <code dangerouslySetInnerHTML={{ __html: html }} />
        </pre>
      </div>
    </div>
  )
}

function escapeHtml(s: string) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
