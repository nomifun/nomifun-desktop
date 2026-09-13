import { readFileSync } from 'node:fs';
import { expect, test } from 'bun:test';

test('the page host and immutable release bootstrap use the same v1 handshake', () => {
  const panel = readFileSync(new URL('./PluginRuntimeSurfacePanel.tsx', import.meta.url), 'utf8');
  const builder = readFileSync(new URL('../../../../../../crates/backend/nomifun-plugin-platform/src/runtime/m1_build.rs', import.meta.url), 'utf8');
  const sdk = readFileSync(new URL('../../../../../../crates/backend/nomifun-plugin-platform/src/assets/product-sdk.js', import.meta.url), 'utf8');
  for (const phase of ['challenge', 'handshake', 'connect']) {
    const event = `nomifun-miniapp-bridge-${phase}-v1`;
    expect(panel).toContain(event);
    expect(builder).toContain(event);
  }
  expect(panel).toContain('nomifun-miniapp-bridge-result-v1');
  expect(sdk).toContain('nomifun-miniapp-bridge-result-v1');
});
