#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MANIFEST = 'crates/backend/nomifun-runtime/components.json';
const NOTICES = 'crates/backend/nomifun-runtime/notices/manifest.json';
const FAMILIES = {
  bun: ['https://github.com/oven-sh/bun', 'bun'],
  node: ['https://nodejs.org', 'node'],
  officecli: ['https://github.com/iOfficeAI/OfficeCLI', '@officecli/officecli'],
};
const DELIVERY = { bun: 'download', node: 'download', officecli: 'embedded' };

export function validateComponents(manifest) {
  if (manifest.schema_version !== 1 || !Array.isArray(manifest.components)) throw new Error('Invalid component catalogue');
  const ids = new Set();
  for (const component of manifest.components) {
    const family = FAMILIES[component.id];
    if (!family || component.source !== family[0] || component.package !== family[1] || ids.has(component.id)) {
      throw new Error(`Component identity is not approved: ${component.id}`);
    }
    ids.add(component.id);
    if (component.delivery !== DELIVERY[component.id]) {
      throw new Error(`Component delivery must match the product contract: ${component.id}`);
    }
    if (!/^\d+\.\d+\.\d+$/.test(component.version) || !component.development_version_requirement || !component.assets?.length) {
      throw new Error(`Missing immutable component contract: ${component.id}`);
    }
    const prefix = component.id === 'node' ? `${component.source}/dist/v${component.version}/`
      : `${component.source}/releases/download/${component.id === 'bun' ? 'bun-' : ''}v${component.version}/`;
    const assets = new Set();
    for (const asset of component.assets) {
      const key = `${asset.target}/${asset.variant}`;
      if (assets.has(key) || !asset.url.startsWith(prefix) || !/^[a-f0-9]{64}$/.test(asset.sha256)
          || /^0+$/.test(asset.sha256) || !['zip', 'tar.gz', 'raw'].includes(asset.format)
          || !/^[a-zA-Z0-9._-]+$/.test(asset.executable)) throw new Error(`Invalid approved asset: ${component.id}/${key}`);
      assets.add(key);
    }
  }
  if (ids.size !== Object.keys(FAMILIES).length) throw new Error('Product component catalogue is incomplete');
  for (const id of ['bun', 'node']) {
    const version = manifest.components.find(component => component.id === id).version;
    const image = manifest.build_images?.[id];
    if (!image?.includes(`:${version}`) || !/@sha256:[a-f0-9]{64}$/.test(image)) throw new Error(`Build image must match approved ${id} version and digest`);
  }
}

export function checkExternalTools(root = ROOT) {
  const read = path => readFileSync(resolve(root, path), 'utf8');
  const manifest = JSON.parse(read(MANIFEST));
  validateComponents(manifest);
  for (const file of ['Dockerfile', 'Dockerfile.prebuilt']) {
    const source = read(file);
    for (const [id, image] of Object.entries(manifest.build_images)) {
      if (file === 'Dockerfile.prebuilt' && id === 'bun') continue;
      if (!source.includes(`ARG ${id.toUpperCase()}_IMAGE="${image}"`)) throw new Error(`${file} diverges from the component catalogue`);
    }
    if (/COPY --from=bun-base .*\/usr\/local\/bin\/bun/.test(source)) throw new Error(`${file} must not duplicate the managed Bun runtime`);
  }
  const notices = JSON.parse(read(NOTICES));
  for (const component of manifest.components) {
    const notice = notices.components.find(entry => entry.id === component.id && entry.version === component.version);
    if (!notice?.files?.length) throw new Error(`Missing redistribution notices for ${component.id}`);
    for (const file of notice.files) {
      if (!/^[a-zA-Z0-9._-]+$/.test(file.filename) || !file.source.startsWith('https://')) throw new Error('Invalid component notice provenance');
      const bytes = readFileSync(resolve(root, dirname(NOTICES), file.filename));
      if (createHash('sha256').update(bytes).digest('hex') !== file.sha256) throw new Error(`Component notice digest mismatch: ${file.filename}`);
    }
  }
  for (const domain of manifest.domain_components ?? []) {
    if (!domain.owner || !existsSync(resolve(root, domain.manifest))) throw new Error(`Unowned component reference: ${domain.id}`);
  }
  const runtime = read('crates/backend/nomifun-runtime/src/extract.rs').split(/#\[cfg\(test\)\]\s*mod\s+tests/)[0];
  if (/(?:symlink|fs::copy)\([^;]*node_filename\(/.test(runtime)) throw new Error('Bun extraction must not publish a Node alias');
  const office = read('crates/backend/nomifun-office/src/artifact.rs');
  if (/CmdBuilder::(?:new|clean_cli)\("(?:officecli|npm)"\)|ChildProcessBuilder::(?:new|clean_cli)\("(?:officecli|npm)"\)/.test(office)) throw new Error('Office must use the verified component supplier');
  for (const retired of ['watch_manager.rs', 'port.rs', 'proxy.rs']) {
    if (existsSync(resolve(root, 'crates/backend/nomifun-office/src', retired))) throw new Error(`Retired Office implementation remains: ${retired}`);
  }
  const officeComponent = manifest.components.find(component => component.id === 'officecli');
  const skillRoot = 'crates/backend/nomifun-skill-library/assets/builtin-skills';
  let runtimeInstructions;
  for (const name of ['officecli', 'officecli-docx', 'officecli-pptx', 'officecli-xlsx', 'officecli-word-form', 'officecli-data-dashboard', 'officecli-financial-model', 'officecli-academic-paper', 'officecli-pitch-deck']) {
    const instructions = read(`${skillRoot}/${name}/references/runtime.md`);
    if (!instructions.includes(officeComponent.version) || !instructions.includes(officeComponent.source) || !instructions.includes(officeComponent.package)
        || !instructions.includes('Host-owned Office execution component') || !instructions.includes('OFFICECLI_NO_AUTO_RESIDENT')) throw new Error(`Office Skill dependency guidance diverges: ${name}`);
    if (runtimeInstructions !== undefined && runtimeInstructions !== instructions) throw new Error('Office Skill runtime instructions must share the same contract');
    runtimeInstructions = instructions;
  }
  const node = read('crates/backend/nomifun-js-runtime/src/lib.rs');
  if (/which::which\("node"\)/.test(node)) throw new Error('Plugin Node must use managed component admission');
  return { components: manifest.components.length, notices: notices.components.length, domain_components: manifest.domain_components?.length ?? 0 };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { console.log(`external-tools boundary passed ${JSON.stringify(checkExternalTools())}`); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
