import { compile } from 'tailwindcss'
import { describe, expect, it } from 'vitest'
import css from '../styles.css?raw'

/**
 * While the interface moves to Tailwind, the legacy stylesheet and the utilities share class names.
 * A legacy class that is also a utility (`.inline`, `.block`, `.table`…) makes Tailwind generate that
 * utility from the old markup, and it beats the legacy rule. This fails as soon as one appears.
 * It goes away with `styles.css`.
 */
describe('legacy class names', () => {
  it('never spell a Tailwind utility', async () => {
    const legacy = [...new Set([...css.matchAll(/\.([a-z][\w-]*)/g)].map((m) => m[1]!))]
    const built = (await compile('@tailwind utilities;')).build(legacy)
    // Every rule Tailwind generated is a legacy name it took for a utility.
    const clashes = [...built.matchAll(/^\.([\w-]+)/gm)].map((m) => m[1]!)
    expect(clashes).toEqual([])
  })
})
