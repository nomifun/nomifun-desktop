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
          || (component.id === 'node' && (asset.format === 'raw' || (asset.target.includes('windows') && !asset.url.endsWith('.zip'))))
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

export function validateExecutionOwners({ managed, hosting, environment }) {
  if (!/ToolId::Node\s+if\s+identity\.name\.as_deref\(\)\s*==\s*Some\("node"\)\s*&&\s*identity\.bun\.is_none\(\)/.test(managed)) {
    throw new Error('Managed external Node must verify genuine Node independently of Bun');
  }
  if (!hosting.includes('nomi_process_runtime') || !hosting.includes('fn shell_command(')
      || !hosting.includes('ChildProcessBuilder::clean_cli(')) {
    throw new Error('Ordinary hosted application commands must use the shared managed process owner');
  }
  if (!environment.includes('process_environment(')
      || !environment.includes('resolve_command_in(')
      || !environment.includes('ChildProcessBuilder::clean_cli(')
      || !/executable\("node",\s*node_path\)/.test(environment)
      || !/executable\("npm",\s*None\)/.test(environment)
      || !/executable\("bun",\s*None\)/.test(environment)) {
    throw new Error('Application environment must probe system/custom Node, npm and Bun separately through the shared process owner and per-launch environment');
  }
}

export function validateOfficeSkillGuidance({ instructions, body, component }) {
  const setup = body.match(/## Managed component setup\n([\s\S]*?)(?:\n## |$)/)?.[1] ?? '';
  if (![component.version, component.source, component.package, 'component=officecli',
    'workspace.process/start', 'literal', 'OFFICECLI_NO_AUTO_RESIDENT=1']
    .every(value => instructions.includes(value))
      || !['component=officecli', 'literal `args`', 'references/runtime.md']
        .every(value => setup.includes(value))) {
    throw new Error('Office Skill guidance must use the approved host component target and literal arguments');
  }
  const retired = /workspace\.process\/exec|Host-owned Office (?:execution )?component|ID appended to the compiled Skill body|nomifun_runtime::resolve_tool|OFFICECLI_EXE|OfficeCliExe|executable_path/;
  if (retired.test(instructions) || retired.test(setup)) {
    throw new Error('Office Skill guidance must not restore hidden component resources or copied executable paths');
  }
  const examples = [...instructions.matchAll(/```json\n([\s\S]*?)\n```/g)].map(match => JSON.parse(match[1]));
  if (examples.length < 2 || examples.some(example => example.component !== component.id
      || !Array.isArray(example.args) || example.args.some(value => typeof value !== 'string')
      || Object.hasOwn(example, 'command') || Object.hasOwn(example, 'cmd') || Object.hasOwn(example, 'env'))) {
    throw new Error('Office Skill examples must select only component=officecli with literal args and host-owned environment');
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
    validateOfficeSkillGuidance({ instructions, body: read(`${skillRoot}/${name}/SKILL.md`), component: officeComponent });
    if (runtimeInstructions !== undefined && runtimeInstructions !== instructions) throw new Error('Office Skill runtime instructions must share the same contract');
    runtimeInstructions = instructions;
  }
  validateExecutionOwners({
    managed: read('crates/backend/nomifun-runtime/src/managed.rs').split(/#\[cfg\(test\)\]\s*mod\s+tests/)[0],
    hosting: read('crates/backend/nomifun-app-hosting/src/owner.rs'),
    environment: read('crates/backend/nomifun-app-hosting/src/environment.rs'),
  });
  return { components: manifest.components.length, notices: notices.components.length, domain_components: manifest.domain_components?.length ?? 0 };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try { console.log(`external-tools boundary passed ${JSON.stringify(checkExternalTools())}`); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
