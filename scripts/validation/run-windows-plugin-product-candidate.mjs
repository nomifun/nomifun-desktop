#!/usr/bin/env bun

/**
 * Installed Windows Plugin/Runtime product acceptance for the current commit.
 *
 * This composes the hardened NSIS install smoke instead of duplicating its
 * install, process-tree, registry, and uninstall logic. The candidate artifact
 * is discovered only inside build.noindex/windows-candidate/<HEAD-short>/.
 *
 * Usage:
 *   bun scripts/validation/run-windows-plugin-product-candidate.mjs --current-candidate
 *   bun scripts/validation/run-windows-plugin-product-candidate.mjs --self-test
 */

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  writeFileSync,
} from 'node:fs';
import { createServer } from 'node:http';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  REPO_ROOT,
  SmokeFailure,
  isPathWithin,
  runCandidateSmoke,
} from './run-windows-desktop-candidate-smoke.mjs';

const PRODUCT_CHECK_TIMEOUT_MS = 6 * 60 * 1000;
const HTTP_TIMEOUT_MS = 30_000;
const POLL_INTERVAL_MS = 250;
const PLUGIN_PACKAGE_ID = 'candidate.product.echo';
const PLUGIN_CAPABILITY_ID = 'candidate.product.echo';
const PLUGIN_CONTRIBUTION_ID = 'capability:candidate.product.echo';
const PLUGIN_ACTION_ID = 'candidate.product.echo.invoke';
const PLUGIN_DESCRIPTION = 'Windows installed product acceptance fixture.';
const MOCK_MODEL = 'candidate-plugin-model';
const INVOKE_MARKER = 'PLUGIN_INSTALLED_INVOKE_OK';

function failure(code, message, details = {}) {
  throw new SmokeFailure(code, message, details);
}

function sleep(milliseconds) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

export function canonicalJson(value) {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  const entries = Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`);
  return `{${entries.join(',')}}`;
}

function envelopeData(body, phase) {
  if (!body || body.success !== true || !('data' in body)) {
    failure('product_api_envelope_invalid', `${phase} returned an invalid API envelope`, {
      phase,
      response_code: typeof body?.code === 'string' ? body.code : null,
    });
  }
  return body.data;
}

async function fetchWithTimeout(url, options, timeoutMs = HTTP_TIMEOUT_MS) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...options, signal: controller.signal, redirect: 'error' });
  } finally {
    clearTimeout(timer);
  }
}

export async function productApi(context, path, options = {}) {
  const method = options.method ?? 'GET';
  const expected = options.expected ?? [200];
  let response;
  try {
    response = await fetchWithTimeout(`${context.getBaseUrl()}${path}`, {
      method,
      headers: {
        authorization: `Bearer ${context.installationToken}`,
        ...(options.body === undefined ? {} : { 'content-type': 'application/json' }),
      },
      ...(options.body === undefined ? {} : { body: JSON.stringify(options.body) }),
    }, options.timeoutMs ?? HTTP_TIMEOUT_MS);
  } catch (error) {
    failure('product_api_transport', `${method} ${path} did not complete within its transport budget`, {
      phase: options.phase ?? path,
      timeout_ms: options.timeoutMs ?? HTTP_TIMEOUT_MS,
      transport_error: error instanceof Error ? error.name : 'unknown',
    });
  }
  const text = await response.text();
  let body = null;
  try {
    body = text.length > 0 ? JSON.parse(text) : null;
  } catch {
    // Shape failure below keeps response bytes out of durable evidence.
  }
  if (!expected.includes(response.status)) {
    failure('product_api_status', `${method} ${path} failed`, {
      phase: options.phase ?? path,
      status_code: response.status,
      response_code: typeof body?.code === 'string' ? body.code : null,
      response_sha256: sha256(text),
    });
  }
  return options.raw ? { status: response.status, body } : envelopeData(body, options.phase ?? path);
}

function assertString(value, code, message) {
  if (typeof value !== 'string' || value.length === 0) failure(code, message);
  return value;
}

function assertInteger(value, code, message) {
  if (!Number.isSafeInteger(value) || value < 0) failure(code, message);
  return value;
}

function buildSourceManifest(versionLabel) {
  const inputSchema = {
    additionalProperties: false,
    properties: { input: {} },
    required: ['input'],
    type: 'object',
  };
  const outputSchema = { type: 'object' };
  const inputRef = `schema://${PLUGIN_PACKAGE_ID}/input@1#${sha256(canonicalJson(inputSchema))}`;
  const outputRef = `schema://${PLUGIN_PACKAGE_ID}/output@1#${sha256(canonicalJson(outputSchema))}`;
  const display = {
    description: PLUGIN_DESCRIPTION,
    name: `Candidate Plugin ${versionLabel}`,
  };
  return canonicalJson({
    build_profile: 'plugin_package_v1',
    config_schema: { additionalProperties: false, type: 'object' },
    contributions: {
      capabilities: [
        {
          config_schema: { additionalProperties: false, type: 'object' },
          conflicts: [],
          contribution_id: PLUGIN_CONTRIBUTION_ID,
          contributions: {
            actions: [
              {
                action_id: PLUGIN_ACTION_ID,
                effect_class: 'pure',
                input_schema: inputRef,
                output_schema: outputRef,
                presentation: 'function_tool',
              },
            ],
            context_schema_refs: [],
            event_schema_refs: [],
            host_ports: [],
            resource_kinds: [],
          },
          display,
          id: PLUGIN_CAPABILITY_ID,
          kind: 'tool',
          package: { id: PLUGIN_PACKAGE_ID, version: '1.0.0' },
          requires: [],
          requires_runtime_features: [],
          supported_platforms: [{ constraint: 'any' }],
          supported_surfaces: ['consumer:agent', 'consumer:gateway', 'desktop'],
          version: '1.0.0',
        },
      ],
      mcp_tools: [],
      skills: [],
    },
    credential_slots: [],
    display,
    entrypoint: 'src/main.js',
    language: 'javascript',
    package_id: PLUGIN_PACKAGE_ID,
    package_version: '1.0.0',
    requires_runtime_features: [],
    schema_version: '1.0.0',
    schemas: {
      [inputRef]: inputSchema,
      [outputRef]: outputSchema,
    },
  });
}

function sourceModule(versionLabel) {
  return `/** @param {PluginActivationContext} context */\nexport async function activate({ mount, sdk }) {\n  void mount;\n  void sdk;\n  return {\n    capabilities: {\n      ${JSON.stringify(PLUGIN_CONTRIBUTION_ID)}: {\n        async invoke({ input }) {\n          return { echo: input, version: ${JSON.stringify(versionLabel)} };\n        },\n      },\n    },\n  };\n}\n`;
}

async function editProject(context, detail, path, content) {
  return productApi(
    context,
    `/api/plugin-projects/${encodeURIComponent(detail.summary.project_id)}/source/edit`,
    {
      method: 'POST',
      phase: `plugin.source.${path}`,
      body: {
        project_id: detail.summary.project_id,
        expected_source_snapshot_digest: assertString(
          detail.source_snapshot_digest,
          'plugin_source_digest_missing',
          'Plugin Project source digest is missing',
        ),
        edit: { kind: 'replace', path, content },
      },
    },
  );
}

async function buildAndTest(context, detail) {
  const projectId = detail.summary.project_id;
  const built = await productApi(
    context,
    `/api/plugin-projects/${encodeURIComponent(projectId)}/build`,
    {
      method: 'POST',
      phase: 'plugin.build',
      timeoutMs: 180_000,
      body: {
        project_id: projectId,
        expected_project_revision: detail.summary.project_revision,
        expected_build_generation: detail.summary.build_generation,
        expected_source_snapshot_digest: assertString(
          detail.source_snapshot_digest,
          'plugin_source_digest_missing',
          'Plugin Project source digest is missing before Build',
        ),
        expected_dependency_lock_digest: assertString(
          detail.dependency_lock_digest,
          'plugin_lock_digest_missing',
          'Plugin Project dependency lock digest is missing before Build',
        ),
      },
    },
  );
  const ready = built.ready;
  if (!ready || !ready.candidate) {
    failure('plugin_ready_candidate_missing', 'Plugin Build did not publish a Ready Candidate');
  }
  let expectedConfigRevision = 0;
  let expectedCredentialBindingsRevision = 0;
  if (built.summary.linked_mount_id) {
    const linkedMount = await productApi(
      context,
      `/api/plugin-mounts/${encodeURIComponent(built.summary.linked_mount_id)}`,
      { phase: 'plugin.test.linked_mount' },
    );
    expectedConfigRevision = assertInteger(
      linkedMount.config?.config_revision,
      'plugin_config_revision_missing',
      'Linked Plugin Mount config revision is missing',
    );
    expectedCredentialBindingsRevision = assertInteger(
      linkedMount.credential_bindings_revision,
      'plugin_credential_revision_missing',
      'Linked Plugin Mount credential revision is missing',
    );
  }
  const tested = await productApi(
    context,
    `/api/plugin-projects/${encodeURIComponent(projectId)}/test`,
    {
      method: 'POST',
      phase: 'plugin.test',
      timeoutMs: 180_000,
      body: {
        project_id: projectId,
        expected_project_revision: built.summary.project_revision,
        expected_build_generation: built.summary.build_generation,
        candidate_id: ready.candidate.candidate_id,
        expected_candidate_digest: ready.candidate.candidate_digest,
        expected_config_revision: expectedConfigRevision,
        expected_credential_bindings_revision: expectedCredentialBindingsRevision,
        resolved_test_input_digest: sha256(canonicalJson({})),
      },
    },
  );
  if (!['passed', 'needs_test_input'].includes(tested.ready?.test?.status)) {
    failure('plugin_candidate_test_failed', 'Plugin Candidate Test did not reach an admissible result', {
      test_status: tested.ready?.test?.status ?? null,
      error_code: tested.ready?.test?.error_code ?? null,
    });
  }
  return tested;
}

class CdpClient {
  constructor(webSocketUrl) {
    this.webSocketUrl = webSocketUrl;
    this.socket = null;
    this.nextId = 1;
    this.pending = new Map();
  }

  async open() {
    const socket = new WebSocket(this.webSocketUrl);
    this.socket = socket;
    await new Promise((resolvePromise, reject) => {
      const timer = setTimeout(() => reject(new Error('CDP WebSocket open timeout')), 10_000);
      socket.addEventListener('open', () => {
        clearTimeout(timer);
        resolvePromise();
      }, { once: true });
      socket.addEventListener('error', () => {
        clearTimeout(timer);
        reject(new Error('CDP WebSocket failed to open'));
      }, { once: true });
    });
    socket.addEventListener('message', (event) => {
      let message;
      try {
        message = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (!Number.isSafeInteger(message.id)) return;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.error) pending.reject(new Error(message.error.message || 'CDP command failed'));
      else pending.resolve(message.result);
    });
  }

  command(method, params = {}, timeoutMs = 30_000) {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error('CDP WebSocket is not open'));
    }
    const id = this.nextId++;
    return new Promise((resolvePromise, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP ${method} timeout`));
      }, timeoutMs);
      this.pending.set(id, { resolve: resolvePromise, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression, timeoutMs = 30_000) {
    const result = await this.command(
      'Runtime.evaluate',
      { expression, awaitPromise: true, returnByValue: true },
      timeoutMs,
    );
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.text || 'page evaluation failed');
    }
    return result.result?.value;
  }

  close() {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error('CDP connection closed'));
    }
    this.pending.clear();
    this.socket?.close();
  }
}

export async function connectToProductPage(context) {
  const response = await fetchWithTimeout(context.getCdpEndpoint(), { method: 'GET' }, 10_000);
  if (response.status !== 200) failure('cdp_targets_unavailable', 'WebView2 target list is unavailable');
  const targets = await response.json();
  const selectedId = context.getCdpTarget()?.id;
  const target = Array.isArray(targets)
    ? targets.find((candidate) => candidate?.id === selectedId)
    : null;
  const webSocketUrl = target?.webSocketDebuggerUrl;
  if (typeof webSocketUrl !== 'string' || webSocketUrl.length === 0) {
    failure('cdp_websocket_missing', 'NomiFun WebView2 target has no debugger WebSocket');
  }
  const client = new CdpClient(webSocketUrl);
  await client.open();
  await client.command('Runtime.enable');
  await client.command('Page.enable');
  return client;
}

export async function pageApi(client, baseUrl, path, options = {}) {
  const request = {
    url: `${baseUrl}${path}`,
    method: options.method ?? 'GET',
    body: options.body,
  };
  const result = await client.evaluate(`(async () => {
    const request = ${JSON.stringify(request)};
    const response = await fetch(request.url, {
      method: request.method,
      headers: request.body === undefined ? {} : {'content-type': 'application/json'},
      body: request.body === undefined ? undefined : JSON.stringify(request.body),
    });
    const text = await response.text();
    let body = null;
    try { body = text.length > 0 ? JSON.parse(text) : null; } catch {}
    return {status: response.status, body};
  })()`, options.timeoutMs ?? HTTP_TIMEOUT_MS);
  const expected = options.expected ?? [200];
  if (!expected.includes(result?.status)) {
    failure('page_api_status', `${request.method} ${path} failed in the installed WebView`, {
      phase: options.phase ?? path,
      status_code: result?.status ?? null,
      response_code: typeof result?.body?.code === 'string' ? result.body.code : null,
    });
  }
  return envelopeData(result.body, options.phase ?? path);
}

function sseFrame(value) {
  return `data: ${JSON.stringify(value)}\n\n`;
}

async function startMockToolProvider() {
  const observations = {
    request_count: 0,
    selected_tool_name: null,
    tool_result_observed: false,
    invoked_version: null,
    error: null,
  };
  const server = createServer(async (request, response) => {
    if (request.method === 'GET' && request.url?.endsWith('/models')) {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ object: 'list', data: [{ id: MOCK_MODEL, object: 'model' }] }));
      return;
    }
    if (request.method !== 'POST' || !request.url?.endsWith('/chat/completions')) {
      response.writeHead(404, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ error: { message: 'not found' } }));
      return;
    }
    try {
      const chunks = [];
      for await (const chunk of request) chunks.push(Buffer.from(chunk));
      const body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      observations.request_count += 1;
      const messages = Array.isArray(body.messages) ? body.messages : [];
      const toolMessage = [...messages].reverse().find((message) => message?.role === 'tool');
      response.writeHead(200, {
        'content-type': 'text/event-stream',
        'cache-control': 'no-cache',
        connection: 'keep-alive',
      });
      if (toolMessage) {
        const content = typeof toolMessage.content === 'string'
          ? toolMessage.content
          : JSON.stringify(toolMessage.content);
        observations.tool_result_observed = content.includes('candidate-product-payload');
        observations.invoked_version = content.includes('candidate-v2') ? 'candidate-v2' : null;
        response.write(sseFrame({
          id: 'candidate-final',
          object: 'chat.completion.chunk',
          created: 1,
          model: MOCK_MODEL,
          choices: [{ index: 0, delta: { role: 'assistant', content: INVOKE_MARKER }, finish_reason: null }],
        }));
        response.write(sseFrame({
          id: 'candidate-final',
          object: 'chat.completion.chunk',
          created: 1,
          model: MOCK_MODEL,
          choices: [{ index: 0, delta: {}, finish_reason: 'stop' }],
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        }));
      } else {
        const tools = Array.isArray(body.tools) ? body.tools : [];
        const tool = tools.find((candidate) =>
          candidate?.type === 'function' &&
          candidate?.function?.parameters?.properties?.input,
        );
        const name = tool?.function?.name;
        if (typeof name !== 'string' || name.length === 0) {
          observations.error = 'plugin_tool_not_advertised';
          response.write(sseFrame({
            id: 'candidate-no-tool',
            object: 'chat.completion.chunk',
            created: 1,
            model: MOCK_MODEL,
            choices: [{ index: 0, delta: { role: 'assistant', content: 'PLUGIN_TOOL_NOT_ADVERTISED' }, finish_reason: 'stop' }],
          }));
        } else {
          observations.selected_tool_name = name;
          response.write(sseFrame({
            id: 'candidate-tool',
            object: 'chat.completion.chunk',
            created: 1,
            model: MOCK_MODEL,
            choices: [{
              index: 0,
              delta: {
                role: 'assistant',
                tool_calls: [{
                  index: 0,
                  id: 'call_candidate_plugin',
                  type: 'function',
                  function: {
                    name,
                    arguments: JSON.stringify({ input: { message: 'candidate-product-payload' } }),
                  },
                }],
              },
              finish_reason: 'tool_calls',
            }],
            usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
          }));
        }
      }
      response.write('data: [DONE]\n\n');
      response.end();
    } catch {
      observations.error = 'mock_provider_request_invalid';
      response.writeHead(400, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ error: { message: 'invalid request' } }));
    }
  });
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolvePromise);
  });
  const address = server.address();
  if (!address || typeof address === 'string') failure('mock_provider_bind_failed', 'Mock provider did not bind');
  return {
    baseUrl: `http://127.0.0.1:${address.port}/v1`,
    observations,
    close: () => new Promise((resolvePromise, reject) =>
      server.close((error) => (error ? reject(error) : resolvePromise()))),
  };
}

async function invokeInstalledPluginThroughAgent(context, expectedVersion) {
  const provider = await startMockToolProvider();
  const client = await connectToProductPage(context);
  try {
    const baseUrl = context.getBaseUrl();
    await pageApi(client, baseUrl, '/api/model-services/free/activate', {
      method: 'POST',
      phase: 'agent.disable_managed_provider',
      body: { enabled: false },
    });
    const createdProvider = await pageApi(client, baseUrl, '/api/providers', {
      method: 'POST',
      expected: [201],
      phase: 'agent.create_mock_provider',
      body: {
        platform: 'stepfun-plan',
        name: 'Installed Plugin Candidate Mock',
        base_url: provider.baseUrl,
        auth_scheme: 'bearer',
        credentials: { api_keys: ['candidate-mock-key'] },
        enabled: true,
        sort_order: 0,
        initial_model: {
          model: MOCK_MODEL,
          enabled: true,
          sort_order: 0,
          capabilities: [{
            task: 'chat',
            traits: ['function_calling', 'streaming'],
            protocol: 'openai.chat_text',
            connection_role: 'default',
            provider_params: { temperature: 0 },
            output_limit: 1024,
          }],
        },
        connections: [],
      },
    });
    const providerId = assertString(
      createdProvider.provider_id,
      'mock_provider_id_missing',
      'Mock provider ID is missing',
    );
    const editor = await pageApi(
      client,
      baseUrl,
      '/api/agent-presets/from-template/chat.minimal',
      {
        method: 'POST',
        phase: 'agent.create_preset',
        body: {
          display_name: 'Installed Plugin Candidate Agent',
          model_route_refs: {},
          chat_route_records: {},
        },
      },
    );
    if (
      editor.revision?.document?.chat_route_records?.agent_chat?.primary?.provider_id !== providerId ||
      editor.revision?.document?.chat_route_records?.agent_chat?.primary?.model !== MOCK_MODEL
    ) {
      failure('mock_provider_not_selected', 'Fresh Agent preset did not select the installed mock provider');
    }
    const presetId = assertString(editor.preset?.preset_id, 'preset_id_missing', 'Agent Preset ID is missing');
    const revision = editor.revision?.reference;
    const draft = structuredClone(editor.draft);
    if (!draft?.document) failure('preset_draft_missing', 'Agent Preset draft is missing');
    const catalog = await pageApi(client, baseUrl, '/api/capabilities', {
      phase: 'agent.capability_catalog',
    });
    const capability = Array.isArray(catalog)
      ? catalog.find((item) => item?.capability?.id === PLUGIN_CAPABILITY_ID)
      : null;
    if (!capability || capability.materialization_state !== 'materialized') {
      failure('plugin_capability_not_materialized', 'Applied Plugin capability is absent from the Agent Catalog');
    }
    draft.document.initial_capabilities = [{
      capability: {
        id: PLUGIN_CAPABILITY_ID,
        version: assertString(
          capability.capability?.version,
          'plugin_capability_version_missing',
          'Applied Plugin capability version is missing',
        ),
      },
      action_allowlist: [],
    }];
    draft.document.on_demand_capabilities = [];
    draft.document.skill_bindings = [];
    draft.document.persona = 'Invoke the only available tool exactly once.';
    draft.document.instructions = 'Always invoke the only supplied function tool, then report its exact result.';
    const preview = await pageApi(
      client,
      baseUrl,
      `/api/agent-presets/${encodeURIComponent(presetId)}/resolve-preview`,
      {
        method: 'POST',
        phase: 'agent.preview_preset',
        body: {
          expected_current_revision: revision,
          draft,
          scene: 'agent_settings',
          surface: 'desktop',
          audience: 'owner',
        },
      },
    );
    if (preview.status !== 'ready' || preview.can_create_session !== true) {
      failure('plugin_agent_preview_not_ready', 'Plugin Agent preview is not ready', {
        diagnostic_code: preview.diagnostics?.[0]?.code ?? null,
      });
    }
    const saved = await pageApi(
      client,
      baseUrl,
      `/api/agent-presets/${encodeURIComponent(presetId)}/revisions`,
      {
        method: 'POST',
        phase: 'agent.save_preset',
        body: {
          expected_current_revision: revision,
          preview_digest: preview.preview_digest,
          draft,
          reason: 'installed Plugin candidate invocation',
        },
      },
    );
    if (!saved.resolved_snapshot_ref) failure('plugin_agent_snapshot_missing', 'Plugin Agent snapshot is missing');
    const session = await pageApi(client, baseUrl, '/api/agent-sessions', {
      method: 'POST',
      phase: 'agent.create_session',
      body: { preset_id: presetId, title: 'Installed Plugin Candidate Invoke' },
    });
    const sessionId = assertString(
      session.agent_session_id,
      'agent_session_id_missing',
      'Agent Session ID is missing',
    );
    await pageApi(
      client,
      baseUrl,
      `/api/agent-sessions/${encodeURIComponent(sessionId)}/turns`,
      {
      method: 'POST',
      phase: 'agent.start_turn',
      timeoutMs: 180_000,
        body: {
          input: { content: 'Invoke the candidate echo tool with candidate-product-payload.' },
          idempotency_key: `candidate-${Date.now()}`,
        },
      },
    );
    const deadline = Date.now() + 120_000;
    let markerObserved = false;
    while (Date.now() < deadline) {
      const messages = await pageApi(
        client,
        baseUrl,
        `/api/agent-sessions/${encodeURIComponent(sessionId)}/messages?after_seq=0&limit=100`,
        { phase: 'agent.poll_messages' },
      );
      markerObserved = JSON.stringify(messages).includes(INVOKE_MARKER);
      if (markerObserved && provider.observations.tool_result_observed) break;
      await sleep(POLL_INTERVAL_MS);
    }
    if (
      !markerObserved ||
      !provider.observations.tool_result_observed ||
      provider.observations.invoked_version !== expectedVersion ||
      provider.observations.error
    ) {
      failure('installed_plugin_invoke_failed', 'Installed Agent did not invoke the exact Plugin Tool', {
        marker_observed: markerObserved,
        tool_result_observed: provider.observations.tool_result_observed,
        invoked_version: provider.observations.invoked_version,
        provider_error: provider.observations.error,
      });
    }
    return {
      provider_request_count: provider.observations.request_count,
      tool_name_sha256: sha256(provider.observations.selected_tool_name),
      marker_observed: true,
      tool_result_observed: true,
      invoked_version: provider.observations.invoked_version,
    };
  } finally {
    client.close();
    await provider.close();
  }
}

async function checkInstallationToken(context) {
  if (typeof context.installationToken !== 'string' || context.installationToken.length < 32) {
    failure('installation_token_not_injected', 'Product runner did not inject an ephemeral installation token');
  }
  const status = await productApi(context, '/api/javascript-runtime/status', {
    phase: 'installation_token.runtime',
  });
  const wrong = await fetchWithTimeout(`${context.getBaseUrl()}/api/javascript-runtime/status`, {
    method: 'GET',
    headers: { authorization: 'Bearer definitely-wrong-installation-token' },
  });
  const scoped = await fetchWithTimeout(`${context.getBaseUrl()}/api/capabilities`, {
    method: 'GET',
    headers: { authorization: `Bearer ${context.installationToken}` },
  });
  if (wrong.status !== 403 || scoped.status !== 403) {
    failure('installation_token_scope_invalid', 'Installation token authentication is not fail-closed and narrow', {
      wrong_token_status: wrong.status,
      non_product_route_status: scoped.status,
    });
  }
  return {
    runtime_status_code: 200,
    wrong_token_status_code: wrong.status,
    non_product_route_status_code: scoped.status,
    selection_revision: status.selection_revision,
  };
}

function compatibleProbe(status, executablePath = null) {
  const probes = Array.isArray(status?.probes) ? status.probes : [];
  return probes.find((probe) =>
    probe?.runtime &&
    probe.compatibility !== 'incompatible' &&
    (executablePath === null || resolve(probe.executable_path) === resolve(executablePath)),
  );
}

async function beginAndCommitRuntime(context, status, runtime, acknowledge) {
  const begun = await productApi(context, '/api/javascript-runtime/switch/begin', {
    method: 'POST',
    phase: 'runtime.switch.begin',
    timeoutMs: 180_000,
    body: {
      expected_selection_revision: status.selection_revision,
      ...(status.selected
        ? {
            expected_selected_runtime_id: status.selected.runtime_installation_id,
            expected_selected_executable_digest: status.selected.executable_digest,
          }
        : {}),
      candidate_runtime_id: runtime.runtime_installation_id,
      expected_candidate_executable_digest: runtime.executable_digest,
      acknowledge_non_recommended_runtime: acknowledge,
    },
  });
  if (
    !begun.requires_switch_decision &&
    begun.pending_candidate === undefined &&
    begun.selected?.runtime_installation_id === runtime.runtime_installation_id
  ) {
    return begun;
  }
  if (!begun.requires_switch_decision || !begun.pending_candidate) {
    failure('runtime_switch_state_invalid', 'Runtime switch returned neither an automatic commit nor a pending decision');
  }
  const committed = await productApi(context, '/api/javascript-runtime/switch/decision', {
    method: 'POST',
    phase: 'runtime.switch.commit',
    timeoutMs: 180_000,
    body: {
      expected_selection_revision: begun.selection_revision,
      candidate_runtime_id: runtime.runtime_installation_id,
      expected_candidate_executable_digest: runtime.executable_digest,
      decision: 'commit_candidate',
    },
  });
  if (
    committed.pending_candidate !== undefined ||
    committed.requires_switch_decision ||
    committed.selected?.runtime_installation_id !== runtime.runtime_installation_id
  ) {
    failure('runtime_switch_commit_failed', 'Runtime Candidate was not committed exactly');
  }
  return committed;
}

async function manualRuntimeProbe(context, status, executablePath) {
  const probed = await productApi(context, '/api/javascript-runtime/probe', {
    method: 'POST',
    phase: 'runtime.probe.manual',
    body: {
      source: 'manual_path',
      expected_selection_revision: status.selection_revision,
      executable_path: executablePath,
    },
  });
  const probe = compatibleProbe(probed, executablePath);
  if (!probe) failure('manual_runtime_probe_failed', 'Copied Node Runtime did not probe as compatible');
  return { status: probed, probe };
}

async function checkRuntimeSwitchRestartFault(context) {
  let status = await productApi(context, '/api/javascript-runtime/status', {
    phase: 'runtime.status.initial',
  });
  let discovered = await productApi(context, '/api/javascript-runtime/probe', {
    method: 'POST',
    phase: 'runtime.probe.auto',
    body: { source: 'auto_discover', expected_selection_revision: status.selection_revision },
  });
  let sourceProbe = compatibleProbe(discovered);
  if (!sourceProbe) failure('system_node_runtime_missing', 'No compatible system Node Runtime was discovered');
  if (!status.selected) {
    status = await beginAndCommitRuntime(
      context,
      discovered,
      sourceProbe.runtime,
      sourceProbe.compatibility !== 'recommended',
    );
    discovered = await productApi(context, '/api/javascript-runtime/probe', {
      method: 'POST',
      phase: 'runtime.probe.selected',
      body: { source: 'auto_discover', expected_selection_revision: status.selection_revision },
    });
    sourceProbe = compatibleProbe(discovered) ?? sourceProbe;
  } else {
    status = discovered;
  }

  const runtimeRoot = join(context.dataRoot, 'candidate-runtime-copies');
  mkdirSync(runtimeRoot, { recursive: true });
  const candidateAPath = join(runtimeRoot, 'candidate-a-node.exe');
  const candidateBPath = join(runtimeRoot, 'candidate-b-node.exe');
  copyFileSync(sourceProbe.executable_path, candidateAPath);
  copyFileSync(sourceProbe.executable_path, candidateBPath);

  const candidateA = await manualRuntimeProbe(context, status, candidateAPath);
  status = await beginAndCommitRuntime(
    context,
    candidateA.status,
    candidateA.probe.runtime,
    candidateA.probe.compatibility !== 'recommended',
  );
  const committedRuntimeId = status.selected.runtime_installation_id;
  const committedDigest = status.selected.executable_digest;
  const firstRestart = await context.restart();
  status = await productApi(context, '/api/javascript-runtime/status', {
    phase: 'runtime.status.after_commit_restart',
  });
  if (
    status.selected?.runtime_installation_id !== committedRuntimeId ||
    status.selected?.executable_digest !== committedDigest
  ) {
    failure('runtime_selection_not_persisted', 'Committed Runtime did not survive Desktop restart');
  }

  const invalidPath = join(runtimeRoot, 'missing-node.exe');
  const invalid = await productApi(context, '/api/javascript-runtime/probe', {
    method: 'POST',
    phase: 'runtime.probe.invalid',
    body: {
      source: 'manual_path',
      expected_selection_revision: status.selection_revision,
      executable_path: invalidPath,
    },
  });
  if (
    invalid.selected?.runtime_installation_id !== committedRuntimeId ||
    !Array.isArray(invalid.probes) ||
    !invalid.probes.some((probe) => probe?.executable_path === invalidPath && probe?.error_code)
  ) {
    failure('runtime_fault_not_contained', 'Invalid Runtime probe changed the committed selection or lost its typed fault');
  }

  const candidateB = await manualRuntimeProbe(context, invalid, candidateBPath);
  const stale = await productApi(context, '/api/javascript-runtime/switch/begin', {
    method: 'POST',
    phase: 'runtime.switch.stale_fault',
    expected: [409],
    raw: true,
    body: {
      expected_selection_revision: candidateB.status.selection_revision - 1,
      expected_selected_runtime_id: committedRuntimeId,
      expected_selected_executable_digest: committedDigest,
      candidate_runtime_id: candidateB.probe.runtime.runtime_installation_id,
      expected_candidate_executable_digest: candidateB.probe.runtime.executable_digest,
      acknowledge_non_recommended_runtime: candidateB.probe.compatibility !== 'recommended',
    },
  });
  if (stale.body?.code !== 'JAVASCRIPT_RUNTIME_STALE') {
    failure('runtime_stale_fault_untyped', 'Stale Runtime switch did not return the revision-conflict code', {
      response_code: stale.body?.code ?? null,
    });
  }
  const recoveryRestart = await context.restart();
  const recovered = await productApi(context, '/api/javascript-runtime/status', {
    phase: 'runtime.status.after_fault_restart',
  });
  if (
    recovered.selected?.runtime_installation_id !== committedRuntimeId ||
    recovered.pending_candidate !== undefined ||
    recovered.requires_switch_decision
  ) {
    failure('runtime_fault_restart_not_stable', 'Rejected Runtime switch changed the committed selection across restart');
  }
  return {
    selected_runtime_id_sha256: sha256(committedRuntimeId),
    executable_digest: committedDigest,
    node_version: recovered.selected.node_version,
    commit_restart_pid_changed: firstRestart.cleanup.root_pid !== firstRestart.pid,
    fault_probe_error_code: invalid.probes.find((probe) => probe?.executable_path === invalidPath)?.error_code,
    fault_restart_pid_changed: recoveryRestart.cleanup.root_pid !== recoveryRestart.pid,
    stale_switch_rejected: true,
  };
}

async function checkPluginLifecycle(context) {
  let library = await productApi(context, '/api/plugins', { phase: 'plugin.library.initial' });
  let project = await productApi(context, '/api/plugin-projects', {
    method: 'POST',
    phase: 'plugin.project.create',
    body: {
      expected_library_revision: library.library_revision,
      package_id: PLUGIN_PACKAGE_ID,
      package_version: '1.0.0',
      display_name: 'Installed Candidate Plugin',
      description: PLUGIN_DESCRIPTION,
      language: 'java_script',
    },
  });
  project = await editProject(context, project, 'nomifun.plugin.json', buildSourceManifest('v1'));
  project = await editProject(context, project, 'src/main.js', sourceModule('candidate-v1'));
  project = await buildAndTest(context, project);
  library = await productApi(context, '/api/plugins', { phase: 'plugin.library.before_initial_apply' });
  let ready = project.ready;
  let mount = await productApi(
    context,
    `/api/plugin-projects/${encodeURIComponent(project.summary.project_id)}/apply`,
    {
      method: 'POST',
      phase: 'plugin.apply.initial',
      timeoutMs: 180_000,
      body: {
        project_id: project.summary.project_id,
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        candidate_id: ready.candidate.candidate_id,
        expected_candidate_digest: ready.candidate.candidate_digest,
        target: { target: 'initial_install', expected_library_revision: library.library_revision },
        allow_breaking: false,
        acknowledge_test_warning: ready.test.status === 'needs_test_input',
      },
    },
  );
  const mountId = mount.summary.mount_id;
  const firstDigest = mount.summary.current?.artifact_digest;
  if (!mountId || !firstDigest || mount.summary.lifecycle !== 'enabled') {
    failure('plugin_initial_apply_failed', 'Initial Plugin Apply did not create an enabled Mount');
  }
  project = await productApi(
    context,
    `/api/plugin-projects/${encodeURIComponent(project.summary.project_id)}`,
    { phase: 'plugin.project.after_initial_apply' },
  );
  project = await editProject(context, project, 'src/main.js', sourceModule('candidate-v2'));
  project = await buildAndTest(context, project);
  ready = project.ready;
  mount = await productApi(context, `/api/plugin-mounts/${encodeURIComponent(mountId)}`, {
    phase: 'plugin.mount.before_replace',
  });
  const replaced = await productApi(
    context,
    `/api/plugin-projects/${encodeURIComponent(project.summary.project_id)}/apply`,
    {
      method: 'POST',
      phase: 'plugin.apply.replace',
      timeoutMs: 180_000,
      body: {
        project_id: project.summary.project_id,
        expected_project_revision: project.summary.project_revision,
        expected_build_generation: project.summary.build_generation,
        candidate_id: ready.candidate.candidate_id,
        expected_candidate_digest: ready.candidate.candidate_digest,
        target: {
          target: 'existing_mount',
          mount_id: mountId,
          expected_mount_revision: mount.summary.mount_revision,
          expected_current_target_digest: mount.summary.current.artifact_digest,
        },
        allow_breaking: false,
        acknowledge_test_warning: ready.test.status === 'needs_test_input',
      },
    },
  );
  const secondDigest = replaced.summary.current?.artifact_digest;
  if (
    !secondDigest ||
    secondDigest === firstDigest ||
    replaced.summary.previous?.artifact_digest !== firstDigest
  ) {
    failure('plugin_replace_rotation_failed', 'Plugin replacement did not rotate current and previous exactly');
  }

  const invocation = await invokeInstalledPluginThroughAgent(context, 'candidate-v2');
  const restored = await productApi(
    context,
    `/api/plugin-mounts/${encodeURIComponent(mountId)}/restore`,
    {
      method: 'POST',
      phase: 'plugin.restore',
      timeoutMs: 180_000,
      body: {
        mount_id: mountId,
        expected_mount_revision: replaced.summary.mount_revision,
        expected_current_target_digest: secondDigest,
        expected_previous_target_digest: firstDigest,
      },
    },
  );
  if (
    restored.summary.current?.artifact_digest !== firstDigest ||
    restored.summary.previous?.artifact_digest !== secondDigest ||
    restored.summary.lifecycle !== 'enabled'
  ) {
    failure('plugin_restore_failed', 'Plugin Restore did not atomically return to the previous Artifact');
  }
  return {
    project_id_sha256: sha256(project.summary.project_id),
    mount_id_sha256: sha256(mountId),
    first_artifact_digest: firstDigest,
    second_artifact_digest: secondDigest,
    candidate_test_status: ready.test.status,
    invoke: invocation,
    restored_to_first_artifact: true,
  };
}

export async function waitForPageSelector(
  client,
  hashRoute,
  selector,
  timeoutMs = 30_000,
  expectedText = null,
) {
  await client.evaluate(`(() => { window.location.hash = ${JSON.stringify(hashRoute)}; return true; })()`);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const state = await client.evaluate(`(() => {
      const element = document.querySelector(${JSON.stringify(selector)});
      return {
        ready: Boolean(element) && (
          ${JSON.stringify(expectedText)} === null ||
          (document.body?.innerText ?? '').includes(${JSON.stringify(expectedText)})
        ),
        text: document.body?.innerText ?? '',
      };
    })()`);
    if (state?.ready) return state;
    await sleep(POLL_INTERVAL_MS);
  }
  failure('desktop_page_timeout', `Installed Desktop route did not render ${selector}`, {
    route: hashRoute,
  });
}

export async function auditInteractiveNames(client, rootSelector) {
  return client.evaluate(`(() => {
    const root = document.querySelector(${JSON.stringify(rootSelector)}) ?? document.body;
    const candidates = [...root.querySelectorAll('button,input,select,textarea,a[href],[role="button"],[role="link"],[tabindex]')];
    const visible = candidates.filter((element) => {
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.visibility !== 'hidden' && style.display !== 'none' && rect.width > 0 && rect.height > 0 && element.tabIndex >= 0;
    });
    const name = (element) => {
      const labelledBy = element.getAttribute('aria-labelledby');
      const labelledText = labelledBy
        ? labelledBy.split(/\\s+/).map((id) => document.getElementById(id)?.textContent ?? '').join(' ')
        : '';
      return [
        element.getAttribute('aria-label'),
        labelledText,
        element.getAttribute('title'),
        element.getAttribute('placeholder'),
        element.textContent,
        element.getAttribute('alt'),
      ].find((value) => typeof value === 'string' && value.trim().length > 0)?.trim() ?? '';
    };
    const missing = visible.filter((element) => name(element).length === 0).map((element) => ({
      tag: element.tagName.toLowerCase(),
      role: element.getAttribute('role'),
      class_name: String(element.className || '').slice(0, 120),
    }));
    return {interactive_count: visible.length, missing};
  })()`);
}

export async function capturePage(client, outputPath) {
  const screenshot = await client.command('Page.captureScreenshot', {
    format: 'png',
    captureBeyondViewport: false,
  });
  const bytes = Buffer.from(screenshot.data, 'base64');
  if (bytes.length < 1_024) failure('desktop_screenshot_invalid', 'Desktop screenshot is unexpectedly small');
  writeFileSync(outputPath, bytes, { flag: 'wx' });
  return {
    path: relative(REPO_ROOT, outputPath).replaceAll('\\', '/'),
    size_bytes: bytes.length,
    sha256: sha256(bytes),
  };
}

async function checkDesktopProductA11y(context) {
  const client = await connectToProductPage(context);
  try {
    const evidenceRoot = join(context.runRoot, 'evidence');
    mkdirSync(evidenceRoot, { recursive: true });
    const plugin = await waitForPageSelector(
      client,
      '/plugins?tab=workshop',
      'aside[aria-label]',
      30_000,
      'Installed Candidate Plugin',
    );
    if (!plugin.text.includes('Installed Candidate Plugin')) {
      failure('plugin_workshop_fixture_missing', 'Installed Plugin Workshop did not render the accepted Project');
    }
    // HubPageShell is a div inside the application layout rather than a
    // nested <main>. The route text above proves the target page is active;
    // audit the whole visible window so global rail controls are covered too.
    const pluginA11y = await auditInteractiveNames(client, 'body');
    if (pluginA11y.interactive_count === 0 || pluginA11y.missing.length > 0) {
      failure('plugin_workshop_a11y_failed', 'Plugin Workshop has unnamed interactive controls', {
        interactive_count: pluginA11y.interactive_count,
        missing: pluginA11y.missing,
      });
    }
    const pluginScreenshot = await capturePage(
      client,
      join(evidenceRoot, 'plugin-workshop.png'),
    );

    const runtime = await waitForPageSelector(
      client,
      '/settings/execution-engines',
      '[data-testid="runtime-manager"]',
    );
    if (!/Node/i.test(runtime.text)) {
      failure('runtime_manager_content_missing', 'Installed Runtime Manager did not render Node Runtime state');
    }
    const runtimeA11y = await auditInteractiveNames(client, '[data-testid="runtime-manager"]');
    if (runtimeA11y.interactive_count === 0 || runtimeA11y.missing.length > 0) {
      failure('runtime_manager_a11y_failed', 'Runtime Manager has unnamed interactive controls', {
        interactive_count: runtimeA11y.interactive_count,
        missing: runtimeA11y.missing,
      });
    }
    const runtimeScreenshot = await capturePage(
      client,
      join(evidenceRoot, 'runtime-manager.png'),
    );
    return {
      plugin_workshop: { ...pluginA11y, screenshot: pluginScreenshot },
      runtime_manager: { ...runtimeA11y, screenshot: runtimeScreenshot },
    };
  } finally {
    client.close();
  }
}

export function resolveCurrentCandidate(repoRoot = REPO_ROOT) {
  const head = spawnSync('git', ['rev-parse', 'HEAD'], {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: false,
    windowsHide: true,
    stdio: 'pipe',
    timeout: 10_000,
  });
  if (head.status !== 0) throw new Error('cannot resolve current Git HEAD');
  const sourceCommit = String(head.stdout).trim().toLowerCase();
  if (!/^[0-9a-f]{40}$/.test(sourceCommit)) throw new Error('current Git HEAD is not canonical');
  const shortCommit = sourceCommit.slice(0, 9);
  const configuredRoot = process.env.NOMIFUN_WINDOWS_CANDIDATE_ROOT;
  const candidateRoot = configuredRoot
    ? resolve(configuredRoot)
    : join(repoRoot, 'build.noindex', 'windows-candidate', shortCommit);
  const allowedRoot = join(repoRoot, 'build.noindex');
  if (!isPathWithin(allowedRoot, candidateRoot) || resolve(candidateRoot) === resolve(allowedRoot)) {
    throw new Error('configured Windows Candidate root must be a child of build.noindex');
  }
  const artifactRoot = join(candidateRoot, 'artifacts');
  const installers = existsSync(artifactRoot)
    ? readdirSync(artifactRoot, { withFileTypes: true })
        .filter((entry) => entry.isFile() && /setup\.exe$/i.test(entry.name))
        .map((entry) => join(artifactRoot, entry.name))
    : [];
  if (installers.length !== 1) {
    throw new Error(`expected exactly one current Candidate setup.exe, found ${installers.length}`);
  }
  return {
    sourceCommit,
    shortCommit,
    installer: installers[0],
    workRoot: join(candidateRoot, 'product-runs'),
  };
}

export function assertSelfTest() {
  const canonical = canonicalJson({ z: 1, a: { y: 2, x: 1 } });
  if (canonical !== '{"a":{"x":1,"y":2},"z":1}') throw new Error('canonical JSON self-test failed');
  const manifest = JSON.parse(buildSourceManifest('self-test'));
  if (
    manifest.build_profile !== 'plugin_package_v1' ||
    manifest.contributions.capabilities[0].contributions.actions[0].action_id !== PLUGIN_ACTION_ID ||
    !manifest.schemas[manifest.contributions.capabilities[0].contributions.actions[0].input_schema]
  ) {
    throw new Error('Plugin source manifest self-test failed');
  }
  const module = sourceModule('candidate-v2');
  if (!module.includes(PLUGIN_CONTRIBUTION_ID) || !module.includes('candidate-v2')) {
    throw new Error('Plugin source module self-test failed');
  }
  return {
    schema_version: '1.0.0',
    status: 'pass',
    suite: {
      name: 'windows-plugin-product-candidate-self-test',
      checks: ['canonical-json', 'source-manifest', 'source-module'],
    },
  };
}

async function runCurrentCandidate() {
  const candidate = resolveCurrentCandidate();
  return runCandidateSmoke({
    installer: candidate.installer,
    sourceCommit: candidate.sourceCommit,
    workRoot: candidate.workRoot,
    productChecks: [
      {
        id: 'installation-token-control-plane',
        timeoutMs: 60_000,
        run: checkInstallationToken,
      },
      {
        id: 'runtime-switch-restart-fault',
        timeoutMs: PRODUCT_CHECK_TIMEOUT_MS,
        run: checkRuntimeSwitchRestartFault,
      },
      {
        id: 'plugin-build-test-apply-invoke-restore',
        timeoutMs: PRODUCT_CHECK_TIMEOUT_MS,
        run: checkPluginLifecycle,
      },
      {
        id: 'plugin-runtime-desktop-a11y',
        timeoutMs: 120_000,
        run: checkDesktopProductA11y,
      },
    ],
  });
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const args = process.argv.slice(2);
  try {
    let result;
    if (args.length === 1 && args[0] === '--self-test') result = assertSelfTest();
    else if (args.length === 1 && args[0] === '--current-candidate') result = await runCurrentCandidate();
    else throw new Error('usage: --self-test | --current-candidate');
    console.log(JSON.stringify(result, null, 2));
    process.exitCode = result.status === 'pass' ? 0 : 1;
  } catch (error) {
    console.log(JSON.stringify({
      schema_version: '1.0.0',
      status: 'fail',
      code: error instanceof SmokeFailure ? error.code : 'runner_error',
      reason: error instanceof Error ? error.message : 'runner failed',
    }, null, 2));
    process.exitCode = 2;
  }
}
