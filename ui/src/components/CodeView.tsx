import hljs from 'highlight.js/lib/core'
import ini from 'highlight.js/lib/languages/ini'
import markdown from 'highlight.js/lib/languages/markdown'
import protobuf from 'highlight.js/lib/languages/protobuf'
import rust from 'highlight.js/lib/languages/rust'
import yaml from 'highlight.js/lib/languages/yaml'
import { useMemo } from 'react'
import { useStore } from '../store'
import { FileTree } from './FileTree'
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

  const paths = useMemo(() => files.map((f) => f.path), [files])

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
      <nav className="code-tree">
        <h3>Fichiers</h3>
        <FileTree paths={paths} active={file?.path ?? null} onSelect={setActive} />
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
