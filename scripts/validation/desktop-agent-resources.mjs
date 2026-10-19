#!/usr/bin/env node
// Disposable real-Desktop T32/T33 sidecar. Only the model is scripted. All
// permission checks, bindings, tools, native children and canonical facts use
// the normal authenticated APIs. The external driver owns exit observations.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, realpath, stat, rename, access } from 'node:fs/promises';
import { constants } from 'node:fs';
import { delimiter, dirname, isAbsolute, join, resolve, basename } from 'node:path';
import { tmpdir } from 'node:os';
import { randomBytes } from 'node:crypto';

const option = (key) => { const index = process.argv.indexOf(key); return index < 0 ? undefined : process.argv[index + 1]; };
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
const started = Date.now();
const rootArgument = option('--root');
assert(rootArgument && isAbsolute(rootArgument), 'Pass the exact absolute --root of the isolated Desktop lifecycle fixture');
const root = await realpath(rootArgument);
const sentinel = (await readFile(join(root, 'fixture-sentinel'), 'utf8')).trim();
assert.equal(basename(root), `nomifun-lifecycle-${sentinel}`);
assert.equal((await realpath(dirname(root))).toLowerCase(), (await realpath(tmpdir())).toLowerCase());
assert.match(sentinel, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
const ready = JSON.parse(await readFile(join(root, 'ready.json'), 'utf8'));
assert.equal(ready.token, sentinel);
assert(Number.isSafeInteger(ready.host_pid) && ready.host_pid > 0);
assert(Number.isSafeInteger(ready.backend_port) && ready.backend_port > 0 && ready.backend_port < 65536);
assert.equal(ready.agent_resources, true, 'The original native fixture must opt into normal owner setup before readiness');
assert.equal((await realpath(ready.data_root)).toLowerCase(), (await realpath(join(root, 'data'))).toLowerCase());
assert.equal(process.platform, 'win32', 'This fixture exercises actual Windows pipe/ConPTY and PowerShell owners');
const base = `http://127.0.0.1:${ready.backend_port}`;
const artifacts = join(root, 'agent-resource-fixtures');
await mkdir(artifacts, { recursive: false });
const evidence = { schema: 'nomifun.desktop-agent-resources.v1', token: sentinel, host_pid: ready.host_pid,
  started_at: new Date().toISOString(), passed: false, stage: 'owner-auth', model_requests: [], api_calls: [] };
const journal = async () => writeFile(join(root, 'agent-resources-progress.json'), `${JSON.stringify(evidence, null, 2)}\n`);
const until = async (check, label, budget = 30_000) => {
  const end = Date.now() + budget;
  while (Date.now() < end) { const value = await check(); if (value) return value; await delay(50); }
  throw new Error(`Timed out waiting for ${label}`);
};
const optionalJson = async (path) => {
  try { return JSON.parse(await readFile(path, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return undefined; throw error; }
};
const existingExecutable = async (name, supplied) => {
  const candidates = supplied ? [supplied] : (process.env.PATH ?? '').split(delimiter).map((entry) => join(entry, name));
  for (const candidate of candidates) {
    try { await access(candidate, constants.R_OK); if ((await stat(candidate)).isFile()) return await realpath(candidate); }
    catch (error) { if (!['ENOENT', 'ENOTDIR', 'EACCES'].includes(error.code)) throw error; }
  }
  throw new Error(`The validation requires an existing ${name}; never install a runtime in this fixture`);
};
let jwt, session, operation, server, failure, closing = false;
const calls = new Map();
const resources = new Map();
const api = async (method, path, body) => {
  const response = await fetch(`${base}${path}`, { method,
    headers: { 'content-type': 'application/json', ...(jwt ? { Authorization: `Bearer ${jwt}` } : {}) },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(20_000) });
  const text = await response.text();
  let value; try { value = JSON.parse(text); } catch { throw new Error(`${method} ${path}: ${response.status}, non-JSON response`); }
  evidence.api_calls.push({ method, path, status: response.status });
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${JSON.stringify(value)}`);
  return value.data ?? value;
};
const toolResult = (body, id) => body.messages?.findLast((message) => message.role === 'tool' && message.tool_call_id === id);
const namedTool = (body, name) => body.tools?.find((tool) => tool.function?.name === name);
const toolCall = (name, id, args) => ({ role: 'assistant', tool_calls: [
  { index: 0, id, type: 'function', function: { name, arguments: JSON.stringify(args) } },
] });
const requireResult = (body, kind) => {
  const id = `desktop-resource-${kind}`;
  const result = toolResult(body, id);
  if (!result) return undefined;
  assert.equal(typeof result.content, 'string');
  assert(!result.content.startsWith('[tool error]'), `The actual ${kind} tool rejected the request: ${result.content}`);
  const native = JSON.parse(result.content);
  assert.equal(native.state, 'running', `The original ${kind} must remain live: ${result.content}`);
  assert(Number.isSafeInteger(native.pid) && native.pid > 0);
  assert.equal(typeof native.process_id, 'string');
  if (resources.has(kind)) assert.deepEqual(resources.get(kind), native, 'A replayed result cannot authorize another native launch');
  else resources.set(kind, native);
  return native;
};
const sendStream = (response, delta, count) => {
  response.writeHead(200, { 'content-type': 'text/event-stream' });
  const chunk = { id: `desktop-lifecycle-${count}`, object: 'chat.completion.chunk', created: 1,
    model: 'desktop-lifecycle-fixture', choices: [{ index: 0, delta, finish_reason: null }] };
  response.end(`data: ${JSON.stringify(chunk)}\n\ndata: ${JSON.stringify({ ...chunk, choices: [{ index: 0, delta: {}, finish_reason: 'tool_calls' }] })}\n\ndata: [DONE]\n\n`);
};
const stopServer = async () => {
  closing = true;
  if (server) { server.closeAllConnections(); await new Promise((done) => server.close(done)); }
};
process.once('SIGTERM', () => { void stopServer(); });
process.once('SIGINT', () => { void stopServer(); });

try {
  // Desktop setup requires the original native host's trust. The debug-only
  // fixture may supply a JWT only after normal owner setup/password login;
  // neither a loopback Origin nor this fixture token is an auth credential.
  const ownerFile = resolve(option('--owner-token-file') ?? join(root, 'agent-resources-owner.jwt'));
  assert(ownerFile.toLowerCase().startsWith(`${root.toLowerCase()}\\`), 'Owner credential must stay inside this exact private fixture');
  jwt = await until(async () => {
    try { return (await readFile(ownerFile, 'utf8')).trim() || undefined; }
    catch (error) { if (error.code === 'ENOENT') return undefined; throw error; }
  }, 'the original native owner login credential', 25_000);
  assert.equal(jwt.split('.').length, 3, 'Expected a normal owner login JWT');
  const owner = await api('GET', '/api/auth/user');
  evidence.owner = { user_id: owner.user?.user_id ?? owner.user_id ?? owner.id, authenticated: true, credential_source: 'normal-native-host-password-login' };
  assert.equal(typeof evidence.owner.user_id, 'string', 'The real authenticated owner identity must be available');
  const auth = JSON.parse(await readFile(join(root, 'agent-resources-auth.json'), 'utf8'));
  assert.equal(auth.token, sentinel); assert.equal(auth.owner_id, evidence.owner.user_id);
  assert.equal(auth.authenticated, true); assert.equal(auth.host_local_setup, true); assert.equal(auth.password_login, true);
  assert.equal(auth.local_trust_exported, false);
  const bun = await existingExecutable('bun.exe', option('--bun-exe') ?? process.env.NOMIFUN_BUN_PATH);
  const powershell = await existingExecutable('powershell.exe', join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe'));
  const processScript = join(artifacts, 'held-agent-process.mjs');
  await writeFile(processScript, `import {writeFileSync,renameSync,appendFileSync} from 'node:fs';
const [marker,token,host,kind]=process.argv.slice(2);
const proof=JSON.stringify({token,host_pid:Number(host),pid:process.pid,kind,image:process.execPath});
appendFileSync(marker+'.launches.jsonl',proof+'\\n');
writeFileSync(marker+'.tmp',proof,{flag:'wx'});
renameSync(marker+'.tmp',marker);
process.stdout.write('RESOURCE_READY:'+kind+':'+process.pid+'\\n');
process.stdin.setEncoding('utf8');
process.stdin.on('data',input=>process.stdout.write('RESOURCE_INPUT:'+input));
process.stdin.resume();setInterval(()=>{},1000);
`, { flag: 'wx' });
  const mcpScript = join(artifacts, 'held-stdio-mcp.ps1');
  const mcpMarker = join(artifacts, 'mcp-pid.json');
  const mcpDescription = `Desktop lifecycle resource hold ${sentinel}`;
  await writeFile(mcpScript, `$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
[Console]::InputEncoding=New-Object Text.UTF8Encoding($false)
[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)
$pending=$null
function Reply($value){[Console]::WriteLine(($value|ConvertTo-Json -Depth 30 -Compress));[Console]::Out.Flush()}
while($null-ne($line=[Console]::ReadLine())){
  $request=ConvertFrom-Json -InputObject $line
  switch([string]$request.method){
    'initialize' {
      Reply @{jsonrpc='2.0';id=$request.id;result=@{protocolVersion=$request.params.protocolVersion;
        capabilities=@{tools=@{}};serverInfo=@{name='desktop-lifecycle-fixture';version='1'}}}
    }
    'tools/list' {
      Reply @{jsonrpc='2.0';id=$request.id;result=@{tools=@(@{name='lifecycle_hold';description=$env:NOMI_FIXTURE_DESCRIPTION;
        inputSchema=@{type='object';additionalProperties=$false;required=@('nonce');
          properties=@{nonce=@{type='string';enum=@($env:NOMI_FIXTURE_TOKEN)}}}})}}
    }
    'tools/call' {
      if($null-ne$pending -or $request.params.name-ne'lifecycle_hold' -or $request.params.arguments.nonce-ne$env:NOMI_FIXTURE_TOKEN){
        throw 'Unexpected or repeated fixture invocation'
      }
      $pending=$request.id
      $marker=@{token=$env:NOMI_FIXTURE_TOKEN;host_pid=[int]$env:NOMI_FIXTURE_HOST;pid=$PID;kind='mcp';request_id=$request.id;
        method=$request.method;tool=$request.params.name;image=[Diagnostics.Process]::GetCurrentProcess().MainModule.FileName}|ConvertTo-Json -Compress
      [IO.File]::AppendAllText(($env:NOMI_FIXTURE_MARKER+'.launches.jsonl'),($marker+[Environment]::NewLine))
      $stream=[IO.File]::Open(($env:NOMI_FIXTURE_MARKER+'.tmp'),[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
      try{$bytes=[Text.Encoding]::UTF8.GetBytes($marker);$stream.Write($bytes,0,$bytes.Length)}finally{$stream.Dispose()}
      [IO.File]::Move(($env:NOMI_FIXTURE_MARKER+'.tmp'),$env:NOMI_FIXTURE_MARKER)
    }
    'notifications/cancelled' {
      if($null-ne$pending -and [string]$request.params.requestId-eq[string]$pending){
        Reply @{jsonrpc='2.0';id=$pending;error=@{code=-32800;message='Original fixture request cancelled'}}
        $pending=$null
      }
    }
    'ping' {Reply @{jsonrpc='2.0';id=$request.id;result=@{}}}
  }
}
`, { flag: 'wx' });
  const transport = { type: 'stdio', command: powershell,
    args: ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', mcpScript],
    env: { NOMI_FIXTURE_TOKEN: sentinel, NOMI_FIXTURE_HOST: String(ready.host_pid), NOMI_FIXTURE_MARKER: mcpMarker, NOMI_FIXTURE_DESCRIPTION: mcpDescription } };
  evidence.stage = 'mcp-configure'; await journal();
  let mcp = await api('POST', '/api/mcp/servers', { name: `Desktop lifecycle ${sentinel}`, transport });
  await api('POST', '/api/mcp/test-connection', { mcp_server_id: mcp.mcp_server_id, name: mcp.name, transport });
  if (!mcp.enabled) mcp = await api('POST', `/api/mcp/servers/${mcp.mcp_server_id}/toggle`, {});
  await api('POST', '/api/mcp/catalog/refresh', {});
  evidence.mcp_server_id = mcp.mcp_server_id;
  const launches = Object.fromEntries(['pipe', 'pty'].map((kind) => [kind, {
    command: bun, args: [processScript, join(artifacts, `${kind}-pid.json`), sentinel, String(ready.host_pid), kind],
    execution_identity: 'current_user', tty: kind === 'pty', wait_ms: 1000,
    ...(kind === 'pty' ? { cols: 100, rows: 30 } : {}),
  }]));
  server = createServer(async (request, response) => {
    if (request.method === 'GET' && request.url === '/v1/models') {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ object: 'list', data: [{ id: 'desktop-lifecycle-fixture', object: 'model' }] })); return;
    }
    if (request.method !== 'POST' || request.url !== '/v1/chat/completions') { response.writeHead(404); response.end(); return; }
    try {
      let bytes = ''; for await (const part of request) { bytes += part; assert(bytes.length < 8 * 1024 * 1024); }
      const body = JSON.parse(bytes);
      const count = evidence.model_requests.length + 1;
      evidence.model_requests.push({ count, call_ids: (body.messages ?? []).filter((item) => item.role === 'tool').map((item) => item.tool_call_id), tools: (body.tools ?? []).map((item) => item.function?.name) });
      await writeFile(join(artifacts, `model-${count}.json`), `${JSON.stringify(body, null, 2)}\n`, { flag: 'wx' });
      assert(count <= 12, 'The fixture must not hide retries/corrections or restart a native task');
      let delta;
      for (const kind of ['pipe', 'pty']) {
        if (requireResult(body, kind)) continue;
        if (namedTool(body, 'start_process')) {
          assert(!calls.has(kind), `The ${kind} launch was already dispatched without its original result`);
          calls.set(kind, count);
          delta = toolCall('start_process', `desktop-resource-${kind}`, launches[kind]);
        } else delta = toolCall('ToolSearch', `find-desktop-resource-${kind}`, { query: 'workspace.process/start' });
        break;
      }
      if (!delta) {
        assert(!toolResult(body, 'desktop-resource-mcp'), 'The intentionally pending original MCP request must not finish before host exit');
        const tool = body.tools?.find((item) => item.function?.parameters?.properties?.nonce?.enum?.includes(sentinel)
          || JSON.stringify(item.function).includes(mcpDescription));
        if (tool) {
          assert(!calls.has('mcp'), 'The original MCP request must be dispatched only once');
          calls.set('mcp', count); evidence.mcp_model_tool = tool.function.name;
          delta = toolCall(tool.function.name, 'desktop-resource-mcp', { nonce: sentinel });
        } else delta = toolCall('ToolSearch', 'find-desktop-resource-mcp', { query: mcpDescription });
      }
      await journal(); sendStream(response, delta, count);
    } catch (error) {
      failure ??= error;
      if (!response.headersSent) response.writeHead(500, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ error: { message: 'Strict Desktop resource fixture failed; inspect its private evidence' } }));
    }
  });
  await new Promise((done, fail) => { server.once('error', fail); server.listen(0, '127.0.0.1', done); });
  evidence.stage = 'agent-create'; await journal();
  const provider = await api('POST', '/api/providers', { platform: 'custom', name: `Desktop lifecycle ${sentinel}`,
    base_url: `http://127.0.0.1:${server.address().port}/v1`, auth_scheme: 'bearer', credentials: { api_keys: [randomBytes(24).toString('hex')] }, enabled: true,
    initial_model: { model: 'desktop-lifecycle-fixture', enabled: true, capabilities: [
      { task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default', provider_params: {} },
    ] } });
  const model = { provider_id: provider.provider_id, model: 'desktop-lifecycle-fixture' };
  const preset = await api('POST', '/api/agent-presets/from-template/coding.codex', { reuse_existing: false, display_name: 'Desktop lifecycle owned resources', model });
  const opened = await api('POST', '/api/agent-sessions', { preset_id: preset.preset.preset_id, model, workspace: join(root, 'work'),
    session_capabilities: { skill_names: [], mcp_server_ids: [mcp.mcp_server_id] },
    resource_selections: [{ resource_kind: 'workspace', resource_id: 'default-workspace' }, { resource_kind: 'process_session', resource_id: 'managed-process-session' }] });
  session = opened.agent_session_id; evidence.session_id = session;
  const accepted = await api('POST', `/api/agent-sessions/${session}/turns`, { idempotency_key: `desktop-resources-${sentinel}`,
    input: { content: 'This is the isolated lifecycle validation. Start the two approved fixture programs exactly once, keep both alive without a task deadline, then invoke the selected lifecycle_hold MCP tool once. Keep its original request pending until the real host exits. Do not complete, retry, cancel, or invent any native result.' } });
  operation = accepted.operation_id; evidence.operation_id = operation;
  evidence.stage = 'original-native-resources-inflight'; await journal();
  const marker = await until(async () => {
    if (failure) throw failure;
    const value = await optionalJson(mcpMarker);
    return resources.size === 2 && value;
  }, 'both original Bun results and the actual inflight MCP request', 45_000);
  const processes = [];
  for (const kind of ['pipe', 'pty', 'mcp']) {
    const path = join(artifacts, `${kind}-pid.json`);
    const nativeMarker = kind === 'mcp' ? marker : JSON.parse(await readFile(path, 'utf8'));
    const launches = (await readFile(`${path}.launches.jsonl`, 'utf8')).trim().split(/\r?\n/).map((line) => JSON.parse(line));
    assert.equal(launches.length, 1, 'A native launch/invocation cannot be hidden behind a replayed ToolResult');
    assert.deepEqual(launches[0], nativeMarker);
    assert.equal(nativeMarker.token, sentinel); assert.equal(nativeMarker.host_pid, ready.host_pid); assert.equal(nativeMarker.kind, kind);
    assert(Number.isSafeInteger(nativeMarker.pid) && nativeMarker.pid > 0);
    if (kind !== 'mcp') assert.equal(nativeMarker.pid, resources.get(kind).pid, 'The ToolResult and native marker must identify the same process');
    else { assert.equal(nativeMarker.method, 'tools/call'); assert.equal(nativeMarker.tool, 'lifecycle_hold'); assert.notEqual(nativeMarker.request_id, undefined); }
    const expected = kind === 'mcp' ? powershell : bun;
    assert.equal((await realpath(nativeMarker.image)).toLowerCase(), expected.toLowerCase());
    processes.push({ pid: nativeMarker.pid, kind, marker_path: path, expected_image: expected,
      call_id: `desktop-resource-${kind}`, ...(kind === 'mcp' ? { mcp_request_id: nativeMarker.request_id } : { process_id: resources.get(kind).process_id }),
      native_tool_result: kind === 'mcp' ? undefined : resources.get(kind) });
  }
  assert.equal(new Set(processes.map((item) => item.pid)).size, 3);
  const canonical = await api('GET', `/api/agent-sessions/${session}`);
  assert(canonical.head.active_turn_id, 'The real Agent and its original MCP call must still be inflight');
  const execution = await api('GET', `/api/agent-sessions/${session}/execution`);
  assert.equal(execution.operation_id, operation); assert.equal(execution.state, 'running');
  const events = await api('GET', `/api/agent-sessions/${session}/events?after_seq=0&limit=500`);
  await writeFile(join(artifacts, 'canonical-inflight.json'), `${JSON.stringify({ canonical, execution, events }, null, 2)}\n`);
  const encoded = JSON.stringify(events);
  for (const kind of ['pipe', 'pty', 'mcp']) assert(encoded.includes(`desktop-resource-${kind}`), 'Each real model call must have canonical event evidence');
  const originalMcp = events.events.filter((event) => event.kind === 'effect/started'
    && event.payload?.storage === 'inline_json' && event.payload.value?.owner_domain === 'mcp'
    && event.payload.value.turn_id === operation && event.payload.value.operation_id.endsWith(':tool:desktop-resource-mcp'));
  assert.equal(originalMcp.length, 1, 'Read the original canonical MCP Effect identity, never infer a new owner from a tool name');
  evidence.mcp_effect_id = originalMcp[0].payload.value.effect_id;
  Object.assign(evidence, { passed: true, stage: 'ready', processes, prepared_in_ms: Date.now() - started,
    ready_at: new Date().toISOString(), canonical_operation_id: canonical.head.active_turn_id,
    agent_binding: canonical.session.agent_binding, original_launch_count: 2, original_mcp_call_count: 1 });
  await journal();
  await writeFile(join(root, 'agent-resources-ready.json.tmp'), `${JSON.stringify(evidence, null, 2)}\n`, { flag: 'wx' });
  await rename(join(root, 'agent-resources-ready.json.tmp'), join(root, 'agent-resources-ready.json'));
  console.log(`NOMIFUN_DESKTOP_AGENT_RESOURCES READY native=3 model_calls=${evidence.model_requests.length} elapsed_ms=${evidence.prepared_in_ms}`);
  const activitySnapshot = async () => {
    const activity = await api('GET', '/api/agent-activities?limit=100');
    return activity.items.filter((item) => item.agent_session_id === session);
  };
  const processTargets = (items) => {
    const unique = new Map();
    for (const resource of items.flatMap((item) => item.resources)) {
      const target = resource.target;
      if (target?.kind === 'process_instance' && target.agent_session_id === session
        && ['pipe', 'pty'].some((kind) => resources.get(kind).process_id === target.process_id)) {
        if (unique.has(target.process_id)) assert.deepEqual(unique.get(target.process_id), target);
        else unique.set(target.process_id, target);
      }
    }
    return [...unique.values()].sort((left, right) => left.process_id.localeCompare(right.process_id));
  };
  const launchCounts = async () => {
    const counts = {};
    for (const kind of ['pipe', 'pty', 'mcp']) {
      const lines = (await readFile(join(artifacts, `${kind}-pid.json.launches.jsonl`), 'utf8')).trim().split(/\r?\n/);
      assert.equal(lines.length, 1, 'Every original native launch/business invocation must still occur exactly once');
      assert.equal(JSON.parse(lines[0]).pid, processes.find((item) => item.kind === kind).pid);
      counts[kind] = lines.length;
    }
    return counts;
  };
  const stopOriginalAgent = async (request) => {
    for (const [key, expected] of Object.entries({ token: sentinel, host_pid: ready.host_pid, session_id: session, operation_id: operation })) {
      assert.equal(request[key], expected, `Stop request ${key} must name this exact original fixture owner`);
    }
    const before = await activitySnapshot();
    const targetsBefore = processTargets(before);
    assert.equal(targetsBefore.length, 2, 'The original two Session-owned processes must be discoverable before Stop');
    const current = before.find((item) => item.target.kind === 'session_turn' && item.target.operation_id === operation && item.can_stop);
    assert(current, 'The user must have a normal host-side Stop target for the exact active Turn');
    const modelBefore = evidence.model_requests.length;
    const launchesBefore = await launchCounts();
    const receipt = await api('POST', '/api/agent-activities/stop', {
      target: current.target, idempotency_key: `stop-original-desktop-resource-agent-${sentinel}`,
    });
    assert.equal(receipt.accepted, true);
    const settled = await until(async () => {
      if (failure) throw failure;
      assert.equal(evidence.model_requests.length, modelBefore, 'Stop and read-only retirement observation cannot issue another model request');
      const [observed, state, activity, page] = await Promise.all([
        api('GET', `/api/agent-sessions/${session}`),
        api('GET', `/api/agent-sessions/${session}/execution`),
        activitySnapshot(), api('GET', `/api/agent-sessions/${session}/events?after_seq=0&limit=500`),
      ]);
      assert.equal(state.operation_id, operation);
      assert.equal(state.automatic_replay_authorized, false);
      const lifecycle = page.events.filter((event) => event.correlation_id === evidence.mcp_effect_id
        && ['effect/uncertain', 'effect/reconciled'].includes(event.kind));
      const uncertainty = lifecycle.findLast((event) => event.kind === 'effect/uncertain');
      const runtime = activity.flatMap((item) => item.resources).filter((resource) => resource.kind === 'runtime');
      if (observed.head.active_turn_id || state.state !== 'cancelled' || state.producer_lease_live
        || state.pending_effects !== 0 || state.unknown_effects !== 1 || !uncertainty
        || !runtime.length || runtime.some((resource) => resource.state !== 'released')) return undefined;
      assert.equal(lifecycle.at(-1).event_id, uncertainty.event_id, 'Native resource proof cannot reconcile the original unknown business outcome');
      assert.equal(uncertainty.payload.storage, 'inline_json');
      const fact = uncertainty.payload.value;
      assert.equal(fact.effect_id, evidence.mcp_effect_id); assert.equal(fact.owner_domain, 'mcp');
      assert.equal(fact.turn_id, operation); assert.equal(fact.operation_id, originalMcp[0].payload.value.operation_id);
      assert.equal(fact.execution.returned, false); assert.equal(fact.execution.cancellation_requested, true);
      assert.equal(fact.cleanup.local, 'confirmed'); assert.equal(fact.cleanup.remote, 'not_required');
      assert.equal(fact.cleanup.error, null);
      const targetsAfter = processTargets(activity);
      assert.deepEqual(targetsAfter, targetsBefore, 'Runtime retirement preserves each exact original Process owner/creator/id');
      const processResources = activity.flatMap((item) => item.resources).filter((resource) => resource.target?.kind === 'process_instance'
        && targetsAfter.some((target) => target.process_id === resource.target.process_id));
      assert(processResources.every((resource) => ['active', 'held_for_user'].includes(resource.state)),
        'The separate Session process owners must remain live and controllable after this Turn Stop');
      return { observed, state, activity, page, targetsAfter, unknown: [{ state: 'unknown', event_id: uncertainty.event_id,
        seq: uncertainty.seq, ...fact }] };
    }, 'the stopped original Turn, released Runtime and preserved original Session processes', 45_000);
    const launchesAfter = await launchCounts();
    assert.deepEqual(launchesAfter, launchesBefore); assert.equal(evidence.model_requests.length, modelBefore);
    const result = { passed: true, token: sentinel, host_pid: ready.host_pid, session_id: session, operation_id: operation,
      runtime_state: 'released', process_targets_before: targetsBefore, process_targets_after: settled.targetsAfter,
      model_requests_before: modelBefore, model_requests_after: evidence.model_requests.length,
      launch_count_before: launchesBefore.pipe + launchesBefore.pty, launch_count_after: launchesAfter.pipe + launchesAfter.pty,
      original_mcp_call_count: launchesAfter.mcp, mcp_unknown_preserved: true, automatic_replay_authorized: settled.state.automatic_replay_authorized,
      unknown_effects: settled.unknown, observed_at: new Date().toISOString() };
    await writeFile(join(root, 'agent-stop-observation.json'), `${JSON.stringify({ ...result, receipt,
      before, after: settled }, null, 2)}\n`, { flag: 'wx' });
    await writeFile(join(root, 'stop-agent-ready.json.tmp'), `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx' });
    await rename(join(root, 'stop-agent-ready.json.tmp'), join(root, 'stop-agent-ready.json'));
    console.log(`NOMIFUN_DESKTOP_AGENT_RESOURCES STOP_READY runtime=released original_processes=2 model_calls=${modelBefore} mcp_business=unknown`);
  };
  // Keep only the original model server alive until the driver ends this exact
  // host. This observation never sends user input, resumes or restarts work.
  let hostClosed = false;
  let stopObserved = false;
  while (!closing) {
    await delay(250);
    if (!hostClosed) {
      if (!stopObserved) {
        const request = await optionalJson(join(root, 'stop-agent.request'));
        if (request) { await stopOriginalAgent(request); stopObserved = true; }
      }
      try { await fetch(`${base}/api/auth/status`, { signal: AbortSignal.timeout(1000) }); }
      catch { hostClosed = true; evidence.host_connection_closed_at = new Date().toISOString(); await journal(); }
    }
  }
} catch (error) {
  evidence.failure = { name: error.name, message: String(error.message).replaceAll(jwt ?? '\0', '[REDACTED]') };
  evidence.stage = `failed:${evidence.stage}`;
  await journal();
  console.error(`NOMIFUN_DESKTOP_AGENT_RESOURCES FAIL ${evidence.failure.message}`);
  process.exitCode = 1;
} finally { await stopServer(); }
