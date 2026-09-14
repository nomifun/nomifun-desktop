import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

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

const prohibitedPatterns = [
  [/\bisMobile\b/g, 'runtime mobile-layout branch'],
  [/\bmobile\b|手机|移动端|\bi(?:Phone|Pad|Pod|OS)\b|\bAndroid\b/g, 'mobile-specific renderer code or copy'],
  [/safe-area-inset|-webkit-touch-callout|-webkit-overflow-scrolling|\b\d+(?:\.\d+)?dvh\b/g, 'mobile viewport or safe-area compatibility'],
  [/@media\s*\(\s*(?:pointer\s*:\s*coarse|hover\s*:\s*none)\s*\)/g, 'input-device-specific media query'],
];

const failures = [];
for (const file of files) {
  const source = readFileSync(file, 'utf8');
  const relativeFile = file.slice(repositoryRoot.length + 1).replaceAll('\\', '/');
  for (const [pattern, reason] of prohibitedPatterns) {
    pattern.lastIndex = 0;
    for (const match of source.matchAll(pattern)) {
      const line = source.slice(0, match.index).split('\n').length;
      failures.push(`${relativeFile}:${line}: ${reason}: ${JSON.stringify(match[0])}`);
    }
  }

  const mediaPattern = /@media\s*\(\s*max-width\s*:\s*(\d+)px\s*\)/g;
  for (const match of source.matchAll(mediaPattern)) {
    const width = Number(match[1]);
    if (width >= supportedMinimumWidth) continue;
    const line = source.slice(0, match.index).split('\n').length;
    failures.push(
      `${relativeFile}:${line}: viewport breakpoint ${width}px is below the supported ${supportedMinimumWidth}px minimum`
    );
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
