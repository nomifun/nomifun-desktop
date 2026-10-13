import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { checkExternalTools, validateComponents, validateExecutionOwners, validateOfficeSkillGuidance } from './check-external-tools.mjs';
const manifest = () => JSON.parse(readFileSync(new URL('../crates/backend/nomifun-runtime/components.json', import.meta.url), 'utf8'));
describe('managed external tool contract', () => {
  test('actual release catalogue, notices and implementation share one contract', () => {
    expect(checkExternalTools().components).toBe(3);
  });
  test('rejects the unrelated same-name Office npm package', () => {
    const value = manifest(); value.components.find(entry => entry.id === 'officecli').package = 'officecli';
    expect(() => validateComponents(value)).toThrow('identity');
  });
  test('keeps Office local and JavaScript engines outside the application payload', () => {
    for (const id of ['bun', 'node', 'officecli']) {
      const value = manifest();
      value.components.find(entry => entry.id === id).delivery = id === 'officecli' ? 'download' : 'embedded';
      expect(() => validateComponents(value)).toThrow('delivery');
    }
  });
  test('rejects an archive from another release and a floating container image', () => {
    const value = manifest(); value.components[0].assets[0].url = 'https://example.com/bun-latest.zip';
    expect(() => validateComponents(value)).toThrow('asset');
    const floating = manifest(); floating.build_images.node = 'node:22';
    expect(() => validateComponents(floating)).toThrow('digest');
  });
  const executionOwners = () => ({
    managed: readFileSync(new URL('../crates/backend/nomifun-runtime/src/managed.rs', import.meta.url), 'utf8').split(/#\[cfg\(test\)\]\s*mod\s+tests/)[0],
    hosting: readFileSync(new URL('../crates/backend/nomifun-app-hosting/src/owner.rs', import.meta.url), 'utf8'),
    environment: readFileSync(new URL('../crates/backend/nomifun-app-hosting/src/environment.rs', import.meta.url), 'utf8'),
  });
  test('rejects Bun admission as managed external Node', () => {
    const sources = executionOwners();
    sources.managed = sources.managed.replace('identity.bun.is_none()', 'identity.bun.is_some()');
    expect(() => validateExecutionOwners(sources)).toThrow('genuine Node');
  });
  test('rejects application command execution outside the shared process owner', () => {
    const sources = executionOwners();
    sources.hosting = sources.hosting.replaceAll('ChildProcessBuilder', 'Command');
    expect(() => validateExecutionOwners(sources)).toThrow('shared managed process owner');
  });
  test('keeps application npm detection independent from Bun', () => {
    const sources = executionOwners();
    sources.environment = sources.environment.replace('executable("npm", None)', 'executable("bun", None)');
    expect(() => validateExecutionOwners(sources)).toThrow('probe system/custom Node, npm and Bun separately');
  });
  const officeGuidance = () => ({
    instructions: readFileSync(new URL('../crates/backend/nomifun-skill-library/assets/builtin-skills/officecli/references/runtime.md', import.meta.url), 'utf8'),
    body: readFileSync(new URL('../crates/backend/nomifun-skill-library/assets/builtin-skills/officecli/SKILL.md', import.meta.url), 'utf8'),
    component: manifest().components.find(entry => entry.id === 'officecli'),
  });
  test('rejects the retired hidden-resource invocation instructions', () => {
    const guidance = officeGuidance();
    guidance.instructions += '\nRead the Host-owned Office execution component and copy executable_path.\n';
    expect(() => validateOfficeSkillGuidance(guidance)).toThrow('hidden component resources');
  });
  test('rejects Office examples mixing a component target with a user executable', () => {
    const guidance = officeGuidance();
    guidance.instructions = guidance.instructions.replace('{"component":"officecli","args"', '{"component":"officecli","command":"officecli","args"');
    expect(() => validateOfficeSkillGuidance(guidance)).toThrow('select only component=officecli');
  });
  test('keeps mandatory component environment in the Host', () => {
    const guidance = officeGuidance();
    guidance.instructions = guidance.instructions.replace('{"component":"officecli","args"', '{"component":"officecli","env":{"OFFICECLI_NO_AUTO_RESIDENT":"0"},"args"');
    expect(() => validateOfficeSkillGuidance(guidance)).toThrow('host-owned environment');
  });
  test('requires the component entry in the Skill setup before the long reference body', () => {
    const guidance = officeGuidance();
    guidance.body = guidance.body.replace('`component=officecli`', '`command=officecli`');
    expect(() => validateOfficeSkillGuidance(guidance)).toThrow('host component target');
  });
});
