#!/usr/bin/env node
// Disposable real-Desktop T32/T33 sidecar. Only the model is scripted. All
// permission checks, bindings, tools, native children and canonical facts use
// the normal authenticated APIs. The external driver owns exit observations.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, realpath, stat, rename, access, readdir } from 'node:fs/promises';
import { constants } from 'node:fs';
import { delimiter, dirname, isAbsolute, join, resolve, basename, relative } from 'node:path';
import { tmpdir } from 'node:os';
import { randomBytes } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { sourceIdentity } from '../lib/source-identity.mjs';

const option = (key) => { const index = process.argv.indexOf(key); return index < 0 ? undefined : process.argv[index + 1]; };
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
const started = Date.now();
const windows = process.platform === 'win32';
const retirementObserveMs = Number(option('--retirement-observe-ms') ?? 0);
assert(Number.isSafeInteger(retirementObserveMs) && (retirementObserveMs === 0
  || (!windows && retirementObserveMs >= 70_000 && retirementObserveMs <= 180_000)),
'The optional macOS read-only fixture observation must be 70000–180000 ms; it is not a product task TTL');
const physicalPath = async (path) => { const physical = await realpath(path); return windows ? physical.toLowerCase() : physical; };
const rootArgument = option('--root');
assert(rootArgument && isAbsolute(rootArgument), 'Pass the exact absolute --root of the isolated Desktop lifecycle fixture');
const root = await realpath(rootArgument);
const sentinel = (await readFile(join(root, 'fixture-sentinel'), 'utf8')).trim();

assert.match(sentinel, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
const ready = JSON.parse(await readFile(join(root, 'ready.json'), 'utf8'));
const assertLinuxFixtureKind = (value) => { const kind = value?.fixture_kind; assert.equal(kind, 'nomifun-global-os-owner', 'A Linux fixture must declare the global-os owner kind'); return kind; };
assert.equal(ready.token, sentinel);
assert.equal(basename(root), `${process.platform === 'linux' ? assertLinuxFixtureKind(ready) : (ready.fixture_kind ?? 'nomifun-lifecycle')}-${sentinel}`);
assert.equal(await physicalPath(dirname(root)), await physicalPath(tmpdir()));
assert(Number.isSafeInteger(ready.host_pid) && ready.host_pid > 0);
assert(Number.isSafeInteger(ready.backend_port) && ready.backend_port > 0 && ready.backend_port < 65536);
assert.equal(ready.agent_resources, true, 'The original native fixture must opt into normal owner setup before readiness');
assert.equal(await physicalPath(ready.data_root), await physicalPath(join(root, 'data')));
assert(['win32', 'darwin', 'linux'].includes(process.platform), 'This fixture exercises actual Desktop pipe/PTY owners on Windows, macOS or Linux');
const base = `http://127.0.0.1:${ready.backend_port}`;
const artifacts = join(root, 'agent-resource-fixtures');
await mkdir(artifacts, { recursive: false });
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
let source;
try { source = await sourceIdentity(repository); } catch (error) { source = { error: String(error?.message ?? error) }; }
const evidence = { schema: 'nomifun.desktop-agent-resources.v1', token: sentinel, host_pid: ready.host_pid,
  started_at: new Date().toISOString(), passed: false, stage: 'owner-auth', model_requests: [], api_calls: [],
  source };
let journalWrites = Promise.resolve();
const journal = () => {
  const bytes = `${JSON.stringify(evidence, null, 2)}\n`;
  // Keep the existing Windows driver's sharing behavior. The macOS reader
  // consumes this live file, so only that path needs an atomic snapshot.
  if (windows) return writeFile(join(root, 'agent-resources-progress.json'), bytes);
  journalWrites = journalWrites.then(async () => {
    await writeFile(join(root, 'agent-resources-progress.json.tmp'), bytes);
    await rename(join(root, 'agent-resources-progress.json.tmp'), join(root, 'agent-resources-progress.json'));
  });
  return journalWrites;
};
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
  // Windows performs first-run setup inside its isolated native debug host.
  // macOS instead uses the existing preparer's fresh owner dataset. Both
  // supply only a normal password-login JWT; local trust never leaves its
  // owner, and neither a loopback Origin nor this marker is a credential.
  const ownerFile = resolve(option('--owner-token-file') ?? join(root, 'agent-resources-owner.jwt'));
  const ownerRelative = relative(root, ownerFile);
  assert(ownerRelative && !ownerRelative.startsWith('..') && !isAbsolute(ownerRelative), 'Owner credential must stay inside this exact private fixture');
  jwt = await until(async () => {
    try { return (await readFile(ownerFile, 'utf8')).trim() || undefined; }
    catch (error) { if (error.code === 'ENOENT') return undefined; throw error; }
  }, 'the original native owner login credential', 25_000);
  assert.equal(jwt.split('.').length, 3, 'Expected a normal owner login JWT');
  const owner = await api('GET', '/api/auth/user');
  evidence.owner = { user_id: owner.user?.user_id ?? owner.user_id ?? owner.id, authenticated: true,
    credential_source: windows ? 'normal-native-host-password-login' : 'isolated-preparer-and-normal-desktop-password-login' };
  assert.equal(typeof evidence.owner.user_id, 'string', 'The real authenticated owner identity must be available');
  const auth = JSON.parse(await readFile(join(root, 'agent-resources-auth.json'), 'utf8'));
  assert.equal(auth.token, sentinel); assert.equal(auth.owner_id, evidence.owner.user_id);
  assert.equal(auth.authenticated, true); assert.equal(auth.host_local_setup, windows); assert.equal(auth.password_login, true);
  if (!windows) assert.equal(auth.isolated_preparer_setup, true);
  assert.equal(auth.local_trust_exported, false);
  const bun = await existingExecutable(windows ? 'bun.exe' : 'bun', option('--bun-exe') ?? process.env.NOMIFUN_BUN_PATH);
  const powershell = windows ? await existingExecutable('powershell.exe', join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe')) : undefined;
  const processScript = join(artifacts, 'held-agent-process.mjs');
  await writeFile(processScript, `import {writeFileSync,renameSync,appendFileSync} from 'node:fs';
const [marker,token,host,kind]=process.argv.slice(2);
const proof=JSON.stringify({token,host_pid:Number(host),pid:process.pid,kind,image:process.execPath,...(process.platform==='darwin'?{parent_pid:process.ppid}:{})});
appendFileSync(marker+'.launches.jsonl',proof+'\\n');
writeFileSync(marker+'.tmp',proof,{flag:'wx'});
renameSync(marker+'.tmp',marker);
process.stdout.write('RESOURCE_READY:'+kind+':'+process.pid+'\\n');
process.stdin.setEncoding('utf8');
process.stdin.on('data',input=>process.stdout.write('RESOURCE_INPUT:'+input));
process.stdin.resume();setInterval(()=>{},1000);
`, { flag: 'wx' });
  const mcpScript = join(artifacts, windows ? 'held-stdio-mcp.ps1' : 'held-stdio-mcp.mjs');
  const mcpMarker = join(artifacts, 'mcp-pid.json');
  const mcpDescription = `Desktop lifecycle resource hold ${sentinel}`;
  await writeFile(mcpScript, windows ? `$ErrorActionPreference='Stop'
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
` : `import {createInterface} from 'node:readline';
import {appendFileSync,writeFileSync,renameSync} from 'node:fs';
let pending,called=false;
const reply=value=>process.stdout.write(JSON.stringify(value)+'\\n');
for await(const line of createInterface({input:process.stdin,terminal:false})){
 const request=JSON.parse(line);
 if(request.method==='initialize')reply({jsonrpc:'2.0',id:request.id,result:{protocolVersion:request.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'desktop-lifecycle-fixture',version:'1'}}});
 else if(request.method==='tools/list')reply({jsonrpc:'2.0',id:request.id,result:{tools:[{name:'lifecycle_hold',description:process.env.NOMI_FIXTURE_DESCRIPTION,inputSchema:{type:'object',additionalProperties:false,required:['nonce'],properties:{nonce:{type:'string',enum:[process.env.NOMI_FIXTURE_TOKEN]}}}}]}});
 else if(request.method==='tools/call'){
  if(called||request.params.name!=='lifecycle_hold'||request.params.arguments.nonce!==process.env.NOMI_FIXTURE_TOKEN)throw Error('Unexpected or repeated fixture invocation');
  called=true;pending=request.id;
  const marker=JSON.stringify({token:process.env.NOMI_FIXTURE_TOKEN,host_pid:Number(process.env.NOMI_FIXTURE_HOST),pid:process.pid,kind:'mcp',request_id:request.id,method:request.method,tool:request.params.name,image:process.execPath});
  appendFileSync(process.env.NOMI_FIXTURE_MARKER+'.launches.jsonl',marker+'\\n');
  writeFileSync(process.env.NOMI_FIXTURE_MARKER+'.tmp',marker,{flag:'wx'});renameSync(process.env.NOMI_FIXTURE_MARKER+'.tmp',process.env.NOMI_FIXTURE_MARKER);
 }else if(request.method==='notifications/cancelled'&&pending!==undefined&&String(request.params.requestId)===String(pending)){
  reply({jsonrpc:'2.0',id:pending,error:{code:-32800,message:'Original fixture request cancelled'}});pending=undefined;
 }else if(request.method==='ping')reply({jsonrpc:'2.0',id:request.id,result:{}});
}
`, { flag: 'wx' });
  const mcpExecutable = windows ? powershell : bun;
  const transport = { type: 'stdio', command: mcpExecutable,
    args: windows ? ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', mcpScript] : [mcpScript],
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
      if (!windows) {
        process.exitCode = 1;
        evidence.failure = { name: error.name, message: String(error.message).replaceAll(jwt ?? '\0', '[REDACTED]') };
        await journal();
      }
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
    if (kind !== 'mcp') {
      if (process.platform === 'darwin' && kind === 'pty') {
        const launcher = resources.get(kind).pid;
        assert.equal(nativeMarker.parent_pid, launcher, 'The actual macOS PTY payload must remain the exact original session launcher child');
        assert.notEqual(nativeMarker.pid, launcher, 'Darwin PTY ToolResult identifies its retained session launcher, not its forked payload');
        assert(isAbsolute(ready.host_program ?? ''), 'The original Darwin launcher executable identity is required');
        processes.push({ pid: launcher, kind: 'pty_launcher', expected_image: await realpath(ready.host_program),
          process_id: resources.get(kind).process_id, payload_pid: nativeMarker.pid,
          native_tool_result: resources.get(kind), identity_source: 'original-tool-result-and-payload-direct-parent-marker' });
      } else assert.equal(nativeMarker.pid, resources.get(kind).pid, 'The ToolResult and native marker must identify the same process');
    }
    else { assert.equal(nativeMarker.method, 'tools/call'); assert.equal(nativeMarker.tool, 'lifecycle_hold'); assert.notEqual(nativeMarker.request_id, undefined); }
    const expected = kind === 'mcp' ? mcpExecutable : bun;
    assert.equal(await physicalPath(nativeMarker.image), await physicalPath(expected));
    processes.push({ pid: nativeMarker.pid, kind, marker_path: path, expected_image: expected,
      call_id: `desktop-resource-${kind}`, ...(kind === 'mcp' ? { mcp_request_id: nativeMarker.request_id } : { process_id: resources.get(kind).process_id }),
      native_tool_result: kind === 'mcp' ? undefined : resources.get(kind) });
  }
  assert.equal(new Set(processes.map((item) => item.pid)).size, process.platform === 'darwin' ? 4 : 3);
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
  console.log(`NOMIFUN_DESKTOP_AGENT_RESOURCES READY native=${processes.length} model_calls=${evidence.model_requests.length} elapsed_ms=${evidence.prepared_in_ms}`);
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
  const observeOriginalRetirement = async ({ settled, targetsBefore, launchesBefore, modelBefore, requestedAt }) => {
    assert.equal(modelBefore, 5, 'The long observation starts after the same original five model requests');
    assert.equal(settled.state.execution_generation, execution.execution_generation);
    const observation = { schema: 'nomifun.macos-original-runtime-retirement-observation.v1',
      session_id: session, operation_id: operation, requested_at: requestedAt,
      fixture_observation_ms: retirementObserveMs, product_timeout_changed: false,
      authority: 'Original registry teardown success plus exact completed-retirement owner native_teardown_proven=true; public APIs and original kernel registrations supply consistency checks',
      instance_identity_scope: 'The original strong retirement handle is bound by Session and operation. The public API exposes no pre-Stop Runtime instance UUID; the single registry instance UUID is associated within this fresh same-generation no-new-Turn case, not claimed as a pre-bound external UUID witness',
      passed: false, samples: [] };
    evidence.retirement_observation = observation;
    const persist = async () => {
      await writeFile(join(root, 'retirement-observation.json.tmp'), `${JSON.stringify(observation, null, 2)}\n`);
      await rename(join(root, 'retirement-observation.json.tmp'), join(root, 'retirement-observation.json'));
      await journal();
    };
    const originalNative = processes.filter(process => process.kind !== 'mcp');
    assert.equal(originalNative.length, 3);
    let nativeIdentities;
    const nativeProof = async () => {
      const text = await readFile(join(root, 'native-observer.jsonl'), 'utf8');
      const frames = text.slice(0, text.lastIndexOf('\n') + 1).split('\n').filter(Boolean).map(line => JSON.parse(line));
      const identities = originalNative.map(process => {
        const registered = frames.filter(frame => frame.event === 'registered' && frame.pid === process.pid);
        assert.equal(registered.length, 1, 'Each retained native generation needs its original kernel registration');
        assert(!frames.some(frame => frame.event === 'exit' && frame.index === registered[0].index),
          'Original Session pipe/PTY generations must stay live throughout the read-only observation');
        return { kind: process.kind, ...registered[0] };
      });
      if (nativeIdentities) assert.deepEqual(identities, nativeIdentities);
      else { nativeIdentities = identities; observation.original_native_generations = identities; }
    };
    const retirementLogs = async () => {
      const records = [];
      for (const name of (await readdir(join(ready.data_root, 'logs'))).filter(name => name.endsWith('.nomicore.log')).sort()) {
        const text = await readFile(join(ready.data_root, 'logs', name), 'utf8');
        for (const line of text.slice(0, text.lastIndexOf('\n') + 1).split('\n').filter(Boolean)) {
          const value = JSON.parse(line), fields = value.fields ?? {};
          if ((fields.conversation_id === session || (fields.session_id === session && fields.operation_id === operation))
            && Date.parse(value.timestamp) >= Date.parse(requestedAt)) records.push({ file: name, ...value });
        }
      }
      const registry = records.filter(value => value.target === 'nomifun_ai_agent::runtime_sessions'
        && value.fields.message === 'Agent Runtime carrier retired after proven native teardown');
      const owners = records.filter(value => value.target?.endsWith('completed_runtime_retirement')
        && value.fields.message === 'completed Runtime retirement owner settled');
      assert(owners.every(value => value.fields.native_teardown_proven === true),
        'Every settlement observer for this unchanged original operation must retain its proven native teardown');
      const warnings = records.filter(value => value.fields.message === 'completed Runtime retirement remains unconfirmed; exact owner retained');
      const missingOwner = records.filter(value => /Original effect resource cleanup owner is unavailable|Runtime retirement has no original resource owner|original Runtime retirement was not confirmed/i.test(JSON.stringify(value.fields)));
      assert.equal(missingOwner.length, 0, 'The exact original retirement must not lose its cleanup owner');
      if (observation.owner_settled) {
        assert.deepEqual(registry, [observation.carrier_retired], 'The bound original carrier retirement proof must remain unique');
        // Terminal relay and direct user Stop can each own an original
        // retirement observer. They may both join the same retained native
        // teardown result; the registry carrier and business calls stay unique.
        assert(owners.length >= observation.owner_settlements.length);
        assert.deepEqual(owners.slice(0, observation.owner_settlements.length), observation.owner_settlements,
          'Previously observed original retirement settlements must remain unchanged');
        observation.owner_settlements = owners;
        assert(!warnings.some(value => Date.parse(value.timestamp) >= Date.parse(observation.owner_settled.timestamp)),
          'The exact original retirement must not become unconfirmed again after successful teardown');
        assert.equal(warnings.length, observation.early_unconfirmed.length, 'No late unconfirmed retry may grow after the original success');
      }
      return { registry, owners, warnings };
    };
    const sample = async () => {
      if (failure) throw failure;
      assert(!closing, 'The observation must complete before fixture shutdown');
      assert.equal(evidence.model_requests.length, modelBefore);
      assert.deepEqual(await launchCounts(), launchesBefore);
      await nativeProof();
      const [observed, state, activity, appended] = await Promise.all([
        api('GET', `/api/agent-sessions/${session}`), api('GET', `/api/agent-sessions/${session}/execution`),
        activitySnapshot(), api('GET', `/api/agent-sessions/${session}/events?after_seq=${settled.observed.head.last_seq}&limit=500`),
      ]);
      assert.equal(observed.head.active_turn_id, undefined);
      assert.equal(state.operation_id, operation); assert.equal(state.execution_generation, execution.execution_generation);
      assert.equal(state.state, 'cancelled'); assert.equal(state.producer_lease_live, false);
      assert.equal(state.pending_effects, 0); assert.equal(state.unknown_effects, 1);
      assert.equal(state.automatic_replay_authorized, false);
      assert.equal(state.model_progress.used_model_steps, settled.state.model_progress.used_model_steps);
      assert(appended.events.length < 500, 'A truncated event page cannot prove the absence of a new Turn or MCP reconciliation');
      assert(!appended.events.some(event => event.kind === 'turn/started'), 'Read-only observation must not create a new Turn');
      assert(!appended.events.some(event => event.correlation_id === evidence.mcp_effect_id && event.kind === 'effect/reconciled'),
        'Local cleanup does not reconcile the original MCP Unknown business outcome');
      const runtime = activity.flatMap(item => item.resources).filter(resource => resource.kind === 'runtime');
      assert(runtime.length && runtime.every(resource => resource.state === 'released'));
      assert.deepEqual(processTargets(activity), targetsBefore);
      const live = activity.flatMap(item => item.resources).filter(resource => targetsBefore.some(target =>
        resource.target?.kind === 'process_instance' && resource.target.process_id === target.process_id));
      assert(live.length >= 2 && live.every(resource => ['active', 'held_for_user'].includes(resource.state)));
      await nativeProof();
      assert.deepEqual(await launchCounts(), launchesBefore);
      assert.equal(evidence.model_requests.length, modelBefore);
      return { observed_at: new Date().toISOString(), monotonic_ns: process.hrtime.bigint().toString(),
        session_last_seq: observed.head.last_seq, execution_generation: state.execution_generation,
        runtime_released: true, original_native_generations_live: 3, original_process_targets_preserved: true,
        model_requests: modelBefore, launches: launchesBefore, mcp_unknown: 1, automatic_replay_authorized: false };
    };
    try {
      await persist();
      await until(async () => {
        const logs = await retirementLogs(); await sample();
        observation.early_unconfirmed = logs.warnings;
        if (!logs.registry.length || !logs.owners.length) return undefined;
        assert.equal(logs.registry.length, 1, 'Exactly one original initialized Runtime carrier must retire');
        assert(logs.owners.length >= 1, 'At least one original completed-retirement observer must settle after proven native teardown');
        assert.equal(typeof logs.registry[0].fields.instance_id, 'string');
        assert(Date.parse(logs.registry[0].timestamp) <= Date.parse(logs.owners[0].timestamp));
        observation.carrier_retired = logs.registry[0]; observation.owner_settled = logs.owners[0];
        observation.owner_settlements = logs.owners;
        // Recheck warnings against the newly bound original success before
        // beginning the fixture window, which crosses the 60s retry backoff.
        await retirementLogs(); return true;
      }, 'the exact original Runtime native teardown and retirement-owner success logs', 45_000);
      observation.observation_started_at = new Date().toISOString();
      const began = process.hrtime.bigint();
      for (;;) {
        observation.samples.push(await sample()); await retirementLogs(); await persist();
        const elapsed = Number(process.hrtime.bigint() - began) / 1_000_000;
        if (elapsed >= retirementObserveMs) break;
        await delay(Math.min(1_000, retirementObserveMs - elapsed));
      }
      // A slow persistence write may cross the deadline. Read the original
      // authorities again after that boundary before claiming the full window.
      observation.samples.push({ ...await sample(), deadline_final_sample: true });
      await retirementLogs();
      observation.elapsed_ms = Number(process.hrtime.bigint() - began) / 1_000_000;
      observation.passed = true; observation.completed_at = new Date().toISOString(); await persist();
      return observation;
    } catch (error) {
      observation.failure = { name: error.name, message: error.message }; await persist(); throw error;
    }
  };
  const stopOriginalAgent = async (request) => {
    for (const [key, expected] of Object.entries({ token: sentinel, host_pid: ready.host_pid, session_id: session, operation_id: operation })) {
      assert.equal(request[key], expected, `Stop request ${key} must name this exact original fixture owner`);
    }
    const before = await activitySnapshot();
    const targetsBefore = processTargets(before);
    stopTargets = targetsBefore;
    assert.equal(targetsBefore.length, 2, 'The original two Session-owned processes must be discoverable before Stop');
    const current = before.find((item) => item.target.kind === 'session_turn' && item.target.operation_id === operation && item.can_stop);
    assert(current, 'The user must have a normal host-side Stop target for the exact active Turn');
    const modelBefore = evidence.model_requests.length;
    const launchesBefore = await launchCounts();
    const requestedAt = new Date().toISOString();
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
    if (retirementObserveMs) result.retirement_observation = await observeOriginalRetirement({
      settled, targetsBefore, launchesBefore, modelBefore, requestedAt });
    await writeFile(join(root, 'stop-agent-ready.json.tmp'), `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx' });
    await rename(join(root, 'stop-agent-ready.json.tmp'), join(root, 'stop-agent-ready.json'));
    console.log(`NOMIFUN_DESKTOP_AGENT_RESOURCES STOP_READY runtime=released original_processes=2 model_calls=${modelBefore} mcp_business=unknown`);
  };
  // Keep only the original model server alive until the driver ends this exact
  // host. This observation never sends user input, resumes or restarts work.
  const stopPipeProcess = async (request, targetsBefore) => {
    for (const [key, expected] of Object.entries({ token: sentinel, host_pid: ready.host_pid, session_id: session, operation_id: operation })) {
      assert.equal(request[key], expected, `Process Stop request ${key} must name this exact original fixture owner`);
    }
    const before = await activitySnapshot();
    const targetsNow = processTargets(before);
    assert.deepEqual(targetsNow, targetsBefore, 'The original pipe and pty process targets must persist after Turn Stop');
    const pipe = targetsNow.find((target) => target.process_id === resources.get('pipe').process_id);
    const pipeItem = before.find((item) => item.target?.kind === 'process_instance' && item.target.process_id === resources.get('pipe').process_id && item.can_stop);
    assert(pipe && pipeItem, 'The original pipe process must still expose its exact ordinary Stop target');
    const modelBefore = evidence.model_requests.length;
    const launchesBefore = await launchCounts();
    const receipt = await api('POST', '/api/agent-activities/stop', {
      target: pipeItem.target, idempotency_key: `stop-original-desktop-resource-pipe-${sentinel}`,
    });
    assert.equal(receipt.accepted, true);
    const settled = await until(async () => {
      if (failure) throw failure;
      const items = await activitySnapshot();
      const rows = items.flatMap((item) => item.resources).filter((resource) => resource.target?.kind === 'process_instance');
      const pipeRows = rows.filter((resource) => resource.target.process_id === resources.get('pipe').process_id);
      const ptyRows = rows.filter((resource) => resource.target.process_id === resources.get('pty').process_id);
      if (pipeRows.some((resource) => resource.state !== 'released')) return undefined;
      assert(ptyRows.length && ptyRows.every((resource) => ['active', 'held_for_user'].includes(resource.state)),
        'Stopping one process must not stop its sibling');
      return { rows, pipe: pipeRows, pty: ptyRows };
    }, 'the exact pipe process release', 45_000);
    const launchesAfter = await launchCounts();
    assert.deepEqual(launchesAfter, launchesBefore); assert.equal(evidence.model_requests.length, modelBefore);
    const after = await activitySnapshot();
    const result = { passed: true, token: sentinel, host_pid: ready.host_pid, session_id: session, operation_id: operation,
      pipe_target: pipe, remaining_process_targets: processTargets(after), pty_state: 'active',
      model_requests_before: modelBefore, model_requests_after: evidence.model_requests.length,
      launch_count_before: launchesBefore.pipe + launchesBefore.pty, launch_count_after: launchesAfter.pipe + launchesAfter.pty,
      observed_at: new Date().toISOString() };
    await writeFile(join(root, 'stop-process-observation.json'), `${JSON.stringify({ ...result, receipt, before, after: settled }, null, 2)}\n`, { flag: 'wx' });
    await writeFile(join(root, 'stop-process-ready.json.tmp'), `${JSON.stringify(result, null, 2)}\n`, { flag: 'wx' });
    await rename(join(root, 'stop-process-ready.json.tmp'), join(root, 'stop-process-ready.json'));
    console.log(`NOMIFUN_DESKTOP_AGENT_RESOURCES PROCESS_STOP_READY pipe=released pty=active model_calls=${modelBefore}`);
  };
  let hostClosed = false;
  let stopObserved = false;
  let stopTargets;
  let stopProcessObserved = false;
  while (!closing) {
    await delay(250);
    if (!windows && failure) throw failure;
    if (!hostClosed) {
      if (!stopObserved) {
        const request = await optionalJson(join(root, 'stop-agent.request'));
        if (request) { await stopOriginalAgent(request); stopObserved = true; }
      } else if (!stopProcessObserved) {
        const request = await optionalJson(join(root, 'stop-process.request'));
        if (request) { await stopPipeProcess(request, stopTargets); stopProcessObserved = true; }
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
