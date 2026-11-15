// Validates an AsyncAPI document with the official parser: `node validate-asyncapi.mjs <file>`.
// Run from the folder where `@asyncapi/parser` is installed (ES modules do not read NODE_PATH).
// Exits 1 on an error; warnings (a newer version of the specification exists) are printed and do not fail.
import { Parser } from '@asyncapi/parser';
import fs from 'node:fs';
const parser = new Parser();
const file = process.argv[2];
const { document, diagnostics } = await parser.parse(fs.readFileSync(file, 'utf8'));
const errors = diagnostics.filter(d => d.severity === 0);
for (const d of diagnostics) console.log(d.severity === 0 ? 'ERROR' : 'warn', d.code, d.path?.join('/'), d.message);
if (!document || errors.length) { console.log('INVALID'); process.exit(1); }
console.log('VALID: channels', document.channels().all().length, 'operations', document.operations().all().length, 'messages', document.messages().all().length);
