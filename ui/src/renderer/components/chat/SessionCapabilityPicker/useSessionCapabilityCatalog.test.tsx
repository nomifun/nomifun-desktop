import '../../../../../test/setup-dom.ts';
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { ipcBridge } from '@/common';
import * as mcpCatalog from '@/renderer/hooks/mcp/catalog';
import { useSessionCapabilityCatalog } from './useSessionCapabilityCatalog';
import { defaultSessionCapabilityDraft } from './model';

const restores: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach((restore) => restore()); });

test('catalog retains unavailable packages for inspection while auto defaults exclude them', async () => {
  const bad = { name: 'bad-auto', description: 'Auto instructions', location: '', is_custom: false, source: 'builtin' as const, session_available: false, session_error: 'Resource exceeds the size limit' };
  const good = { name: 'good-auto', description: 'Valid instructions', location: '', is_custom: false, source: 'builtin' as const };
  track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([bad, good]));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([bad, good]));
  track(spyOn(mcpCatalog, 'ensureBackendMcpCatalog').mockResolvedValue({ allServers: [], builtinServers: [], userServers: [] }));
  const hook = renderHook(() => useSessionCapabilityCatalog());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  expect(hook.result.current.error).toBeUndefined();
  expect(hook.result.current.catalog.skills).toHaveLength(2);
  expect(hook.result.current.catalog.skills.find((skill) => skill.name === 'bad-auto')?.session_error).toBe(bad.session_error);
  expect(defaultSessionCapabilityDraft(hook.result.current.catalog).skillNames).toEqual(['good-auto']);
});
