import '../../../test/setup-dom.ts';
import { expect, test } from 'bun:test';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { appHosting } from './appHostingBridge';

test('ZIP import sends original binary bytes and existing LocalTrust over real HTTP', async () => {
  const bytes = new Uint8Array([0x50, 0x4b, 0x03, 0x04, 0x00, 0xff, 0x18]);
  let seenPath = '', seenTrust = '', seenType = '';
  let received = new Uint8Array();
  const server = createServer(async (request, response) => {
    response.setHeader('Access-Control-Allow-Origin', '*');
    response.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
    response.setHeader('Access-Control-Allow-Headers', 'content-type, x-nomi-local-trust');
    if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return; }
    const url = new URL(request.url!, 'http://127.0.0.1');
    seenPath = `${url.pathname}?${url.searchParams}`;
    seenTrust = String(request.headers['x-nomi-local-trust'] ?? '');
    seenType = String(request.headers['content-type'] ?? '');
    const chunks: Uint8Array[] = [];
    for await (const chunk of request) chunks.push(chunk as Uint8Array);
    received = Buffer.concat(chunks);
    response.writeHead(200, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ success: true, data: { id: 'imported', name: 'Uploaded project' } }));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const previousPort = window.__backendPort, previousTrust = window.__nomiLocalTrust;
  window.__backendPort = (server.address() as AddressInfo).port;
  window.__nomiLocalTrust = 'per-boot-test-connection';
  try {
    const result = await appHosting.importZip(new File([bytes], 'project.zip'), 'source', 'Uploaded project');
    expect(seenPath).toBe('/api/apps/import-zip?import_kind=source&name=Uploaded+project');
    expect(seenTrust).toBe('per-boot-test-connection');
    expect(seenType).toBe('application/zip');
    expect([...received]).toEqual([...bytes]);
    expect(result.id).toBe('imported');
  } finally {
    if (previousPort === undefined) delete window.__backendPort; else window.__backendPort = previousPort;
    if (previousTrust === undefined) delete window.__nomiLocalTrust; else window.__nomiLocalTrust = previousTrust;
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});

test('app workspace metadata, Session and same-app import use the authenticated owner routes', async () => {
  const requests: Array<{ path: string; method: string; body: unknown; trust: string }> = [];
  const server = createServer(async (request, response) => {
    response.setHeader('Access-Control-Allow-Origin', '*');
    response.setHeader('Access-Control-Allow-Methods', 'POST, PUT, OPTIONS');
    response.setHeader('Access-Control-Allow-Headers', 'content-type, x-nomi-local-trust');
    if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return; }
    const chunks: Uint8Array[] = [];
    for await (const chunk of request) chunks.push(chunk as Uint8Array);
    const body = JSON.parse(Buffer.concat(chunks).toString());
    requests.push({ path: request.url!, method: request.method!, body, trust: String(request.headers['x-nomi-local-trust']) });
    response.writeHead(200, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ success: true, data: request.url?.endsWith('/development-session') ? {
      app: { id: 'same-app', development_session_id: '0190f5fe-7c00-7a00-8000-000000000301' },
      conversation: { conversation_id: '0190f5fe-7c00-7a00-8000-000000000301', type: 'nomi', name: 'Application development', created_at: 1, modified_at: 1,
        model: { provider_id: '0190f5fe-7c00-7a00-8000-000000000303', model: 'chat-model' },
        extra: { workspace: 'C:/apps/same-app', official_template_key: 'app.developer' } },
    } : { id: 'same-app', ...(body.completion ? { completion_observation: { ...body.completion, files_changed: true } } : {}) } }));
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const previousPort = window.__backendPort, previousTrust = window.__nomiLocalTrust;
  window.__backendPort = (server.address() as AddressInfo).port; window.__nomiLocalTrust = 'workspace-test-trust';
  try {
    await appHosting.create.invoke({ name: 'Todo', description: 'Task reminders', tags: ['Productivity'] });
    await appHosting.update.invoke({ id: 'same-app', name: 'Todo', description: 'Updated', tags: ['Life'], settings: { project_directory: '.', environment: {} } });
    const opened = await appHosting.developmentSession.invoke({ id: 'same-app', model: { provider_id: 'configured-provider', model: 'chat-model' } });
    expect(opened.conversation.id).toBe('0190f5fe-7c00-7a00-8000-000000000301');
    expect(opened.conversation).not.toHaveProperty('conversation_id');
    expect(opened.conversation.model.id).toBe('0190f5fe-7c00-7a00-8000-000000000303');
    expect(opened.conversation.model.use_model).toBe('chat-model');
    expect(opened.conversation.extra.workspace).toBe('C:/apps/same-app');
    await appHosting.importInto.invoke({ id: 'same-app', source: 'C:/work/project', import_kind: 'source' });
    const completion = { session_id: '0190f5fe-7c00-7a00-8000-000000000301', turn_id: '0190f5fe-7c00-7a00-8000-000000000304' };
    const configured = await appHosting.update.invoke({ id: 'same-app', detect_settings: true, completion });
    expect(configured.id).toBe('same-app');
    expect(configured.completion_observation).toEqual({ ...completion, files_changed: true });
    expect(requests.map((request) => [request.method, request.path])).toEqual([
      ['POST', '/api/apps'], ['PUT', '/api/apps/same-app'], ['POST', '/api/apps/same-app/development-session'], ['POST', '/api/apps/same-app/import'],
      ['PUT', '/api/apps/same-app'],
    ]);
    expect(requests[0].body).toEqual({ name: 'Todo', description: 'Task reminders', tags: ['Productivity'] });
    expect(requests[1].body).toEqual({ name: 'Todo', description: 'Updated', tags: ['Life'], settings: { project_directory: '.', environment: {} } });
    expect(requests[2].body).toEqual({ model: { provider_id: 'configured-provider', model: 'chat-model' } });
    expect(requests[3].body).toEqual({ source: 'C:/work/project', import_kind: 'source' });
    expect(requests[4].body).toEqual({ detect_settings: true, completion });
    expect(requests.every((request) => request.trust === 'workspace-test-trust')).toBe(true);
    expect(requests.every((request) => !JSON.stringify(request.body).includes('workspace-test-trust'))).toBe(true);
  } finally {
    if (previousPort === undefined) delete window.__backendPort; else window.__backendPort = previousPort;
    if (previousTrust === undefined) delete window.__nomiLocalTrust; else window.__nomiLocalTrust = previousTrust;
    server.closeAllConnections(); await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});

test('project-session pagination and idempotent creation use the same owner API and ordinary Conversation mapper', async () => {
  const session = '0190f5fe-7c00-7a00-8000-000000000301';
  const requests: Array<{ path: string; body: unknown; trust: string }> = [];
  const conversation = { conversation_id: session, type: 'nomi', name: 'Project iteration', created_at: 1, modified_at: 2,
    model: { provider_id: '0190f5fe-7c00-7a00-8000-000000000303', model: 'chat-model' }, extra: { workspace: 'C:/apps/project' } };
  const server = createServer(async (request, response) => {
    response.setHeader('Access-Control-Allow-Origin', '*'); response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    response.setHeader('Access-Control-Allow-Headers', 'content-type, x-nomi-local-trust');
    if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return; }
    const chunks: Uint8Array[] = []; for await (const chunk of request) chunks.push(chunk as Uint8Array);
    requests.push({ path: request.url!, body: chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : undefined, trust: String(request.headers['x-nomi-local-trust']) });
    response.writeHead(200, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ success: true, data: request.method === 'GET' ? { items: [{ conversation, editable_in_app: false, archived: true }], total: 4, has_more: true,
      next_cursor: 'next:cursor', project_directory: 'C:/apps/project', current_session_id: session } : { app: { id: 'project' }, conversation } }));
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const previousPort = window.__backendPort, previousTrust = window.__nomiLocalTrust;
  window.__backendPort = (server.address() as AddressInfo).port; window.__nomiLocalTrust = 'project-test-trust';
  try {
    const page = await appHosting.developmentSessions.invoke({ id: 'project', cursor: 'older:session/2', limit: 30 });
    expect(page.items[0].conversation.id).toBe(session); expect(page.items[0].archived).toBe(true); expect(page.items[0].conversation).not.toHaveProperty('archived');
    expect(page.items[0].editable_in_app).toBe(false); expect(page.next_cursor).toBe('next:cursor');
    const creation_key = '0190f5fe-7c00-7a00-8000-000000000304';
    await appHosting.developmentSession.invoke({ id: 'project', new_session: true, creation_key, model: { provider_id: 'provider', model: 'model' } });
    await appHosting.developmentSession.invoke({ id: 'project', session_id: session, refresh_workflow: true });
    expect(requests.map(request => request.path)).toEqual(['/api/apps/project/development-sessions?cursor=older%3Asession%2F2&limit=30', '/api/apps/project/development-session', '/api/apps/project/development-session']);
    expect(requests[1].body).toEqual({ new_session: true, creation_key, model: { provider_id: 'provider', model: 'model' } });
    expect(requests[2].body).toEqual({ session_id: session, refresh_workflow: true });
    expect(requests.every(request => request.trust === 'project-test-trust')).toBe(true);
  } finally {
    if (previousPort === undefined) delete window.__backendPort; else window.__backendPort = previousPort;
    if (previousTrust === undefined) delete window.__nomiLocalTrust; else window.__nomiLocalTrust = previousTrust;
    server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve()));
  }
});
