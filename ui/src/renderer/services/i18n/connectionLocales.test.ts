/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import enSettings from './locales/en-US/settings.json';
import zhSettings from './locales/zh-CN/settings.json';

type LocaleJson = Record<string, unknown>;

const SRC_ROOT = fileURLToPath(new URL('../../..', import.meta.url));
const KEY_PATTERN = /\bsettings\.(connections|deviceConnections)\.([A-Za-z0-9_]+)\b/g;

/** `settings.connections` is the model-provider connection profile namespace;
 * `settings.deviceConnections` is the phone/relay hub namespace. They collided
 * once — every literal key used in sources must exist in both locales. */
function* walk(dir: string): Generator<string> {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(path);
    else if (/\.(ts|tsx)$/.test(entry.name) && !entry.name.endsWith('.d.ts') && !entry.name.includes('.test.')) yield path;
  }
}

function getLocaleValue(locale: LocaleJson, namespace: string, key: string): unknown {
  const ns = locale[namespace];
  if (!ns || typeof ns !== 'object') return undefined;
  return (ns as LocaleJson)[key];
}

describe('connection locale coverage', () => {
  test('every settings.(connections|deviceConnections) key used in sources resolves in both locales', () => {
    const usages = new Map<string, Set<string>>();
    for (const file of walk(SRC_ROOT)) {
      const source = readFileSync(file, 'utf8');
      for (const match of source.matchAll(KEY_PATTERN)) {
        const ref = `${match[1]}.${match[2]}`;
        if (!usages.has(ref)) usages.set(ref, new Set());
        usages.get(ref)!.add(file);
      }
    }
    // Non-vacuous: the walk must actually collect keys from both namespaces.
    expect(usages.has('connections.hasCredentials')).toBe(true);
    expect(usages.has('deviceConnections.title')).toBe(true);
    const failures: string[] = [];
    for (const [name, locale] of [
      ['en-US', enSettings as unknown as LocaleJson],
      ['zh-CN', zhSettings as unknown as LocaleJson],
    ] as const) {
      for (const [ref, files] of usages) {
        const dot = ref.indexOf('.');
        const value = getLocaleValue(locale, ref.slice(0, dot), ref.slice(dot + 1));
        if (typeof value !== 'string' || !value.trim()) {
          for (const file of files) failures.push(`${name} settings.${ref} <- ${file}`);
        }
      }
    }
    expect(failures).toEqual([]);
  });

  test('the two connection namespaces stay distinct and the profile keys exist', () => {
    for (const locale of [enSettings, zhSettings] as const) {
      expect(locale.connections.title).not.toBe(locale.deviceConnections.title);
      for (const key of ['hasCredentials', 'noCredentials', 'keepCredentialsHint'] as const) {
        const value = (locale.connections as unknown as LocaleJson)[key];
        expect(typeof value === 'string' && value.trim().length > 0).toBe(true);
      }
    }
  });
});
