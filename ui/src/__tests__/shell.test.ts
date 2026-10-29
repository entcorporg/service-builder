import { describe, expect, it } from 'vitest'
import { shellQuote } from '../shell'

describe('shellQuote', () => {
  it('leaves plain paths alone', () => {
    expect(shellQuote('/home/me/projets/demo.sb.json')).toBe('/home/me/projets/demo.sb.json')
    expect(shellQuote('mcp')).toBe('mcp')
  })
  it('quotes spaces, placeholders and shell metacharacters', () => {
    expect(shellQuote('/home/me/Mes Projets/a.sb.json')).toBe("'/home/me/Mes Projets/a.sb.json'")
    expect(shellQuote('<projet.sb.json>')).toBe("'<projet.sb.json>'")
    expect(shellQuote('a;b$c')).toBe("'a;b$c'")
  })
  it('escapes single quotes', () => {
    expect(shellQuote("l'été")).toBe("'l'\\''été'")
  })
})
