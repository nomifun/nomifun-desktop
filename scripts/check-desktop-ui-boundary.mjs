import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inspectRendererSource } from './desktop-ui-boundary-policy.mjs';

const repositoryRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
const rendererRoot = join(repositoryRoot, 'ui', 'src', 'renderer');
const supportedMinimumWidth = 880;
const sourceExtensions = new Set(['.css', '.html', '.json', '.ts', '.tsx']);
const files = [];

const collectSources = (directory) => {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) collectSources(path);
    else if (entry.isFile() && sourceExtensions.has(extname(entry.name))) files.push(path);
  }
};

collectSources(rendererRoot);
files.push(
  join(repositoryRoot, 'ui', 'index.html'),
  join(repositoryRoot, 'ui', 'public', 'manifest.webmanifest')
);

const failures = [];
for (const file of files) {
  const source = readFileSync(file, 'utf8');
  const relativeFile = file.slice(repositoryRoot.length + 1).replaceAll('\\', '/');
  for (const { line, reason, value } of inspectRendererSource(relativeFile, source, supportedMinimumWidth)) {
    failures.push(`${relativeFile}:${line}: ${reason}: ${JSON.stringify(value)}`);
  }
}

const baseCss = readFileSync(
  join(rendererRoot, 'styles', 'themes', 'base.css'),
  'utf8'
);
if (!baseCss.includes(`--app-min-width: ${supportedMinimumWidth}px;`)) {
  failures.push(`ui/src/renderer/styles/themes/base.css: --app-min-width must remain ${supportedMinimumWidth}px`);
}

const desktopMain = readFileSync(join(repositoryRoot, 'apps', 'desktop', 'src', 'main.rs'), 'utf8');
if (!desktopMain.includes(`.min_inner_size(${supportedMinimumWidth}.0, 600.0)`)) {
  failures.push(`apps/desktop/src/main.rs: desktop minimum size must remain ${supportedMinimumWidth}x600`);
}

if (failures.length > 0) {
  console.error('Desktop UI boundary check failed:\n');
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log(`Desktop UI boundary OK (${files.length} renderer sources, minimum ${supportedMinimumWidth}x600).`);
