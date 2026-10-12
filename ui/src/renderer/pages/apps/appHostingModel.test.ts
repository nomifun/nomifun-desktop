import { expect, test } from 'bun:test';
import { canEmbedAppUrl, canOpenAppUrl, editableAppSettings, parseAppEnvironment } from './appHostingModel';

test('remote WebUI blocks Host loopback URLs while desktop and local WebUI can open them', () => {
  for (const url of ['http://localhost:4317', 'http://127.0.0.1:4317', 'http://127.8.0.1', 'http://[::1]:4317', 'http://app.localhost:4317']) {
    expect(canOpenAppUrl(url, false, '192.168.1.22')).toBe(false);
    expect(canOpenAppUrl(url, true, 'tauri.localhost')).toBe(true);
    expect(canOpenAppUrl(url, false, 'localhost')).toBe(true);
  }
  expect(canOpenAppUrl('https://my-app.example.com', false, '192.168.1.22')).toBe(true);
  for (const url of ['javascript:alert(1)', 'file:///app/index.html', '//localhost:4317', 'not a URL', '']) {
    expect(canOpenAppUrl(url, true, 'localhost')).toBe(false);
  }
});

test('inline preview excludes renderer origin, credentials and inaccessible remote loopback', () => {
  expect(canEmbedAppUrl('http://localhost:8787/api/apps', false, 'http://localhost:8787/apps')).toBe(false);
  expect(canEmbedAppUrl('http://user:secret@localhost:4317', true, 'http://localhost:8787/apps')).toBe(false);
  expect(canEmbedAppUrl('http://localhost:4317', false, 'http://192.168.1.22:8787/apps')).toBe(false);
  expect(canEmbedAppUrl('http://localhost:4317', true, 'http://tauri.localhost/apps')).toBe(true);
  expect(canEmbedAppUrl('https://app.example.com', false, 'http://192.168.1.22:8787/apps')).toBe(true);
});

test('environment parsing retains values and rejects attempts to override Host connection fields', () => {
  expect(parseAppEnvironment('PORT=4317\nDATA_PATH=./local data\nKEY=abc=123\nEMPTY=\n')).toEqual({ PORT: '4317', DATA_PATH: './local data', KEY: 'abc=123', EMPTY: '' });
  expect(() => parseAppEnvironment('NOMIFUN_LOCAL_TRUST=my-token')).toThrow('prepared by the Host');
  expect(() => parseAppEnvironment('invalid name=value')).toThrow('Invalid environment');
  expect(() => parseAppEnvironment('NO_EQUALS')).toThrow('Invalid environment');
});

test('editable settings exclude internal connection and Session owner fields', () => {
  const value = editableAppSettings({ project_directory: '.', environment: { PORT: '4317' },
    agent_binding_id: 'internal', development_session_id: 'canonical-development-session' } as Parameters<typeof editableAppSettings>[0]);
  expect(value).not.toHaveProperty('agent_binding_id');
  expect(value).not.toHaveProperty('development_session_id');
});
