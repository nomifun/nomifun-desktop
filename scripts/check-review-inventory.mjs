// Read-only consistency check for the persistent audit inventory.
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const directories = (relative) => readdirSync(resolve(root, relative), { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => relative + '/' + entry.name + '/');
const expected = [
  ...['crates/agent', 'crates/backend', 'crates/shared'].flatMap(directories)
    .filter((directory) => readdirSync(resolve(root, directory)).includes('Cargo.toml')),
  ...directories('apps').filter((directory) => readdirSync(resolve(root, directory)).includes('Cargo.toml')),
  ...directories('ui/src/common'),
  'ui/src/platform/',
  ...directories('ui/src/renderer').filter((directory) => !directory.endsWith('/pages/')),
  ...directories('ui/src/renderer/pages'),
];
const ledger = readFileSync(resolve(root, 'docs/reviews/audit-progress.zh.md'), 'utf8');
const registered = [...ledger.matchAll(/^\| \x60([^\x60]+\/)\x60 \|/gm)].map((match) => match[1]);
const issues = [];
const taskIds = [...ledger.matchAll(/^\| (R\d+-\d+[a-z]?) \|/gm)].map((match) => match[1]);
for (const taskId of new Set(taskIds)) {
  if (taskIds.filter((entry) => entry === taskId).length !== 1) {
    issues.push(taskId + ': duplicate issue ID in the audit ledger');
  }
}
for (const directory of expected) {
  const count = registered.filter((entry) => entry === directory).length;
  if (count !== 1) issues.push(directory + ': expected one entry, found ' + count);
}
for (const directory of new Set(registered)) {
  if (!expected.includes(directory)) issues.push(directory + ': no longer in the module inventory');
}
if (issues.length) {
  console.error(issues.join('\n'));
  process.exitCode = 1;
} else {
  console.log('Audit inventory OK: ' + expected.length + ' module boundaries, ' + taskIds.length + ' unique issue IDs; no missing or duplicate entries.');
}
