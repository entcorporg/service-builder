import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { describe, it } from 'node:test'
import { bump, bumpVersion, currentVersion, detectLevel, setVersion } from './release.mjs'

describe('detectLevel', () => {
  it('reads the three mentions, case-insensitively', () => {
    assert.equal(detectLevel('feat: sql block [minor]'), 'minor')
    assert.equal(detectLevel('feat: tables view [Normal]'), 'normal')
    assert.equal(detectLevel('breaking: new file format [MAJOR]'), 'major')
  })
  it('returns null without mention, and ignores look-alikes', () => {
    assert.equal(detectLevel('fix: typo'), null)
    assert.equal(detectLevel('see [normally] and (major)'), null)
    assert.equal(detectLevel('chore(release): v0.2.0 [skip ci]'), null)
  })
  it('keeps the strongest when a push carries several commits', () => {
    assert.equal(detectLevel('a [minor]\n\nb [major]\n\nc [normal]'), 'major')
    assert.equal(detectLevel('a [minor]\n\nb [normal]'), 'normal')
  })
})

describe('bumpVersion', () => {
  it('raises the number the type names and resets those on its right', () => {
    assert.equal(bumpVersion('0.1.0', 'normal'), '0.2.0')
    assert.equal(bumpVersion('0.1.4', 'normal'), '0.2.0')
    assert.equal(bumpVersion('0.1.4', 'minor'), '0.1.5')
    assert.equal(bumpVersion('0.0.0', 'minor'), '0.0.1')
    assert.equal(bumpVersion('1.4.9', 'major'), '2.0.0')
    assert.equal(bumpVersion('0.9.9', 'major'), '1.0.0')
  })
  it('rejects what is not a plain X.Y.Z or a known type', () => {
    assert.throws(() => bumpVersion('1.0', 'minor'))
    assert.throws(() => bumpVersion('1.0.0-rc.1', 'minor'))
    assert.throws(() => bumpVersion('1.0.0', 'patch'))
  })
})

describe('bump', () => {
  const fixture = () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'sb-release-'))
    const put = (file, content) => {
      fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true })
      fs.writeFileSync(path.join(dir, file), content)
    }
    put('Cargo.toml', '[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n\n[workspace.dependencies]\nserde = { version = "1" }\n')
    put('src-tauri/tauri.conf.json', '{\n  "productName": "X",\n  "version": "0.1.0",\n  "identifier": "x"\n}\n')
    put('package.json', '{\n  "name": "x",\n  "version": "0.1.0"\n}\n')
    put('ui/package.json', '{\n  "name": "ui",\n  "version": "0.1.0",\n  "dependencies": { "a": "^0.1.0" }\n}\n')
    put(
      'Cargo.lock',
      'version = 4\n\n[[package]]\nname = "a"\nversion = "0.1.0"\ndependencies = [\n "serde",\n]\n\n[[package]]\nname = "serde"\nversion = "0.1.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "abc"\n\n[[package]]\nname = "b"\nversion = "0.1.0"\n',
    )
    return { dir, read: (f) => fs.readFileSync(path.join(dir, f), 'utf8') }
  }

  it('moves every version file together: 0.1.0 + normal = 0.2.0', () => {
    const { dir, read } = fixture()
    assert.equal(bump('normal', dir), '0.2.0')
    assert.equal(currentVersion(dir), '0.2.0')
    assert.match(read('Cargo.toml'), /\[workspace\.package\]\nversion = "0\.2\.0"\nedition/)
    assert.match(read('src-tauri/tauri.conf.json'), /"version": "0\.2\.0",\n  "identifier"/)
    assert.match(read('ui/package.json'), /"version": "0\.2\.0",\n  "dependencies": \{ "a": "\^0\.1\.0" \}/)
    fs.rmSync(dir, { recursive: true })
  })

  it('updates workspace crates in Cargo.lock and leaves registry crates alone', () => {
    const { dir, read } = fixture()
    bump('minor', dir)
    const lock = read('Cargo.lock')
    assert.match(lock, /name = "a"\nversion = "0\.1\.1"/)
    assert.match(lock, /name = "b"\nversion = "0\.1\.1"/)
    assert.match(lock, /name = "serde"\nversion = "0\.1\.0"\nsource/)
    fs.rmSync(dir, { recursive: true })
  })

  it('sets an exact version everywhere, including a lower one', () => {
    const { dir, read } = fixture()
    bump('major', dir)
    assert.equal(setVersion('0.1.0', dir), '0.1.0')
    assert.equal(currentVersion(dir), '0.1.0')
    assert.match(read('Cargo.lock'), /name = "a"\nversion = "0\.1\.0"/)
    assert.throws(() => setVersion('latest', dir))
    fs.rmSync(dir, { recursive: true })
  })

  it('refuses to bump when the files already disagree', () => {
    const { dir } = fixture()
    fs.writeFileSync(path.join(dir, 'package.json'), '{ "version": "0.3.0" }')
    assert.throws(() => bump('minor', dir), /disagree/)
    fs.rmSync(dir, { recursive: true })
  })
})

describe('repository', () => {
  it('has every version file in sync', () => {
    assert.match(currentVersion(), /^\d+\.\d+\.\d+$/)
  })
})
