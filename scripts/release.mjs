// Release helper used by CI (.github/workflows/ci.yml), runnable locally with Node >= 20 and no dependency.
//
//   node scripts/release.mjs detect < text    prints major | normal | minor, or nothing when the text has no mention
//   node scripts/release.mjs bump <level>     rewrites every version file and prints the new version
//   node scripts/release.mjs check            fails unless every version file carries the same version
//
// The update type is mentioned as [minor], [normal] or [major] and maps to one number of X.Y.Z:
//   minor  -> 0.0.X      normal -> 0.X.0      major -> X.0.0
// Raising a number resets the ones on its right: 0.1.4 + [normal] = 0.2.0.
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

export const LEVELS = ['minor', 'normal', 'major']

/** The strongest `[minor]`, `[normal]` or `[major]` mentioned in `text`, or null. */
export function detectLevel(text) {
  const found = [...text.matchAll(/\[(minor|normal|major)\]/gi)].map((m) => m[1].toLowerCase())
  return found.sort((a, b) => LEVELS.indexOf(b) - LEVELS.indexOf(a))[0] ?? null
}

export function bumpVersion(version, level) {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(version)
  if (!m) throw new Error(`not a X.Y.Z version: ${version}`)
  const [major, normal, minor] = m.slice(1).map(Number)
  switch (level) {
    case 'major':
      return `${major + 1}.0.0`
    case 'normal':
      return `${major}.${normal + 1}.0`
    case 'minor':
      return `${major}.${normal}.${minor + 1}`
    default:
      throw new Error(`unknown update type "${level}" (expected ${LEVELS.join(', ')})`)
  }
}

// Each version lives in one place per file; `read` extracts it and `write` replaces it.
const WORKSPACE_VERSION = /(\[workspace\.package\][^[]*?\nversion = ")([^"]+)(")/
const JSON_VERSION = /("version": ")([^"]+)(")/

const FILES = [
  { file: 'Cargo.toml', pattern: WORKSPACE_VERSION },
  { file: 'src-tauri/tauri.conf.json', pattern: JSON_VERSION },
  { file: 'package.json', pattern: JSON_VERSION },
  { file: 'ui/package.json', pattern: JSON_VERSION },
]

function readVersion(dir, { file, pattern }) {
  const m = pattern.exec(fs.readFileSync(path.join(dir, file), 'utf8'))
  if (!m) throw new Error(`no version found in ${file}`)
  return m[2]
}

/** The version of every tracked file, by path. */
export function versions(dir = root) {
  return Object.fromEntries(FILES.map((f) => [f.file, readVersion(dir, f)]))
}

export function currentVersion(dir = root) {
  const all = versions(dir)
  const distinct = new Set(Object.values(all))
  if (distinct.size !== 1) {
    throw new Error(`version files disagree: ${Object.entries(all).map(([f, v]) => `${f}=${v}`).join(', ')}`)
  }
  return [...distinct][0]
}

/** Workspace crates are the lockfile packages without a `source`; they follow the workspace version. */
function bumpLockfile(text, from, to) {
  return text
    .split('\n\n')
    .map((block) =>
      block.startsWith('[[package]]') && !/^source = /m.test(block) ? block.replace(`version = "${from}"`, `version = "${to}"`) : block,
    )
    .join('\n\n')
}

/** Rewrites every version file in `dir`; returns the new version. */
export function bump(level, dir = root) {
  const from = currentVersion(dir)
  const to = bumpVersion(from, level)
  for (const { file, pattern } of FILES) {
    const p = path.join(dir, file)
    fs.writeFileSync(p, fs.readFileSync(p, 'utf8').replace(pattern, `$1${to}$3`))
  }
  const lock = path.join(dir, 'Cargo.lock')
  if (fs.existsSync(lock)) fs.writeFileSync(lock, bumpLockfile(fs.readFileSync(lock, 'utf8'), from, to))
  return to
}

function main([command, arg]) {
  switch (command) {
    case 'detect': {
      const level = detectLevel(fs.readFileSync(0, 'utf8'))
      if (level) console.log(level)
      return
    }
    case 'bump':
      console.log(bump(arg))
      return
    case 'check':
      console.log(currentVersion())
      return
    default:
      throw new Error('usage: release.mjs detect | bump <minor|normal|major> | check')
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2))
  } catch (e) {
    console.error(e.message)
    process.exit(1)
  }
}
