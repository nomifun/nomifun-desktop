/** Isolated desktop fixture. Production process components, read-only API mocks. */
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import '@arco-design/web-react/es/_util/react-19-adapter';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/pages/conversation/Messages/messages.css';
import zh from '../src/renderer/services/i18n/locales/zh-CN';
import type { IMessageThinking, IMessageToolCall } from '../src/common/chat/chatLib';
import { parseConversationId, parseExecutionAttemptId } from '../src/common/types/ids';
import { insertDelegationRecords } from '../src/renderer/pages/conversation/Messages/delegationProcessModel';
import { compactFileActivityItems } from '../src/renderer/pages/conversation/Messages/fileActivityModel';
import { isTaskControlReceipt } from '../src/renderer/pages/conversation/Messages/toolMessageVisibility';

globalThis.fetch = (async input => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href);
  return url.pathname === '/api/settings/client'
    ? new Response(JSON.stringify({ success: true, data: {} }), { headers: { 'Content-Type': 'application/json' } })
    : new Response(JSON.stringify({ success: false, error: 'This preview only exposes read fixtures.' }), { status: 403, headers: { 'Content-Type': 'application/json' } });
}) as typeof fetch;
const { ipcBridge } = await import('../src/common');
const { database } = await import('../src/common/adapter/ipcBridge');
const { configService } = await import('../src/common/config/configService');
const { setBrowserStorageGeneration, initializeAgentBrowserStorageGeneration } = await import('../src/common/utils/browserStorageKey');
setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000004');
initializeAgentBrowserStorageGeneration(6);
const { leadConversation, leadConversationId, childConversationId, requestId, makeStep, makeAttempt, makeDetail, makeDelegationRecord } = await import('./fixtures/conversationDelegation');
const steps = ['game-designer', 'ui-ux-designer', 'game-developer', 'tech-architect', 'Synthesize results']
  .map((title, index) => makeStep(index, { title, status: index < 2 ? 'completed' : 'pending' }));
const attempts = steps.slice(0, 2).map((step, index) => makeAttempt(step, { status: 'completed',
  attempt_id: parseExecutionAttemptId(`00000000-09c4-7000-8000-${String(30 + index).padStart(12, '0')}`),
  conversation_id: index === 0 ? childConversationId : parseConversationId('00000000-09c4-7000-8000-000000000005') }));
const detail = makeDetail({ steps, attempts });
ipcBridge.agentExecution.get.invoke = async () => detail;
ipcBridge.agentExecution.events.changed.on = () => () => {};
ipcBridge.agentExecution.events.leadThinking.on = () => () => {};
ipcBridge.conversation.reconnected.on = () => () => {};
ipcBridge.conversation.get.invoke = async ({ conversation_id }) => {
  const attempt = attempts.find(attempt => attempt.conversation_id === conversation_id);
  return attempt ? { ...leadConversation, id: conversation_id, execution_step_id: attempt.step_id, execution_attempt_id: attempt.attempt_id } : leadConversation;
};
database.getConversationMessages.invoke = async () => ({ items: [], total: 0, has_more: false });
const cleanupRetried = new Set<string>();
const retainedResources = (attemptId: string) => [{ kind: 'runtime' as const, state: cleanupRetried.has(attemptId) ? 'released' as const : 'unconfirmed' as const,
  detail: cleanupRetried.has(attemptId) ? null : '执行已结束，资源释放凭据尚待核对。' }];
ipcBridge.agentActivities.list.invoke = async request => {
  const attempt = attempts.find(attempt => attempt.attempt_id === request.attempt_id && attempt.step_id === request.step_id);
  const step = steps.find(step => step.step_id === attempt?.step_id);
  return { items: request.execution_id === detail.execution.execution_id && attempt && step ? [{
    key: `preview-exact-attempt:${attempt.attempt_id}`, title: step.title, kind: 'delegated_task', state: 'completed', agent_session_id: attempt.conversation_id,
    target: { kind: 'execution_task', execution_id: detail.execution.execution_id, step_id: step.step_id, attempt_id: attempt.attempt_id,
      expected_execution_version: detail.execution.version, expected_step_version: step.version },
    continuation: 'none', waiting_reason: null, stop_accepted: true, execution_stopped: true, resources: retainedResources(attempt.attempt_id),
    can_stop: false, can_retry_cleanup: !cleanupRetried.has(attempt.attempt_id), updated_at_ms: null,
  }] : [], next_cursor: null };
};
ipcBridge.agentActivities.stop.invoke = async request => {
  if (request.target.kind !== 'execution_task') throw new Error('This fixture only exposes exact task cleanup.');
  cleanupRetried.add(request.target.attempt_id);
  return { accepted: true, execution_stopped: true, resources: retainedResources(request.target.attempt_id) };
};
for (const event of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'responseStream'] as const) ipcBridge.conversation[event].on = () => () => {};
ipcBridge.agentPlatform.sessions.onAgentChanged.on = () => () => {};
ipcBridge.conversation.turnFiles.invoke = async () => [];
ipcBridge.fileStream.contentUpdate.on = () => () => {};
ipcBridge.knowledge.onTreeChanged.on = () => () => {};
ipcBridge.knowledge.onEntryContentUpdated.on = () => () => {};
ipcBridge.workspaceOfficeWatch.start.invoke = async () => {};
ipcBridge.workspaceOfficeWatch.stop.invoke = async () => {};
ipcBridge.workspaceOfficeWatch.fileAdded.on = () => () => {};
ipcBridge.fs.listWorkspaceFiles.invoke = async () => [];
await configService.initialize();

database.getToolDetails.invoke = async request => {
  const command = "git status --short\nrg -n 'DEFAULT_CANVAS_GRID_STEP|creative-canvas-grid-size' ui/src/renderer\nbun run check:desktop-ui-boundary";
  const input = request.call_id === 'command' ? command : request.call_id === 'file-read' ? '{"path":"src/CanvasSurface.tsx","offset":1,"limit":80}'
    : request.call_id === 'file-write' ? '{"path":"src/CanvasSurface.test.tsx","patch":"@@ -1 +1 @@\\n-old value\\n+new value"}' : '{"query":"设计方案","workspace":"当前项目"}';
  const output = request.call_id === 'command' ? ' M ui/src/renderer/CanvasSurface.tsx\nCanvasSurface.tsx:17:const DEFAULT_CANVAS_GRID_STEP = 18;\n桌面 UI 边界检查通过。'
    : request.call_id === 'file-read' ? 'export const DEFAULT_CANVAS_GRID_STEP = 18;'
      : request.call_id === 'file-write' ? '文件已更新。' : '{"results":[{"title":"下拉菜单交互","summary":"已找到相关设计说明"}],"total":1}';
  return { turn_id: request.turn_id, call_id: request.call_id, call_event_id: `canonical-${request.call_id}`, through_seq: 50,
    result_seq: 45, section: request.section, availability: 'available', content: request.section === 'arguments' ? input : output,
    format: request.section === 'arguments' && request.call_id === 'command' ? 'text' : request.call_id === 'generic' || request.section === 'arguments' ? 'json' : 'text',
    offset: 0, total_bytes: new TextEncoder().encode(request.section === 'arguments' ? input : output).length, eof: true, non_text_parts: 0,
    ...(request.section === 'arguments' && request.call_id === 'command' ? { working_directory: '/workspace/nomifun' } : {}) };
};
const { default: ProcessTraceItem } = await import('../src/renderer/pages/conversation/Messages/components/ProcessTraceItem');
const { default: TurnProcessDisclosure } = await import('../src/renderer/pages/conversation/Messages/components/TurnProcessDisclosure');
const { ToolDetailsProvider } = await import('../src/renderer/pages/conversation/Messages/components/ToolDetails');
const { default: DelegationRecordRow } = await import('../src/renderer/pages/conversation/Messages/components/DelegationRecordRow');
const { ExecutionProvider } = await import('../src/renderer/pages/conversation/execution/ExecutionContext');
const { default: ExecutionContentSwitcher } = await import('../src/renderer/pages/conversation/execution/ExecutionContentSwitcher');
const { ConversationProvider } = await import('../src/renderer/hooks/context/ConversationContext');
const tools: IMessageToolCall[] = [
  ['command', 'start_process', '检查改动并验证桌面交互边界'], ['file-read', 'read_file', 'src/CanvasSurface.tsx'],
  ['file-read-part2', 'read_file', 'src/CanvasSurface.tsx'], ['file-read-input', 'read_file', 'src/input.ts'],
  ['file-write', 'apply_patch', 'src/CanvasSurface.test.tsx'], ['generic', 'mcp__design__search_documents__abcdefghijklmnop', '查找下拉菜单交互规范'],
].map(([call_id, name, description]) => ({ id: call_id, type: 'tool_call', conversation_id: leadConversationId,
  turn_id: requestId, position: 'left', content: { call_id, name, description, status: 'completed', result_seq: 45,
    timeline: { through_seq: 50, segments: [{ segment_id: call_id, group_id: 'canonical', kind: 'tool', call_id, first_seq: 5, last_seq: 45, state: 'completed' }] } } }));
for (const tool of tools) {
  if (tool.content.name === 'read_file') Object.assign(tool.content, { capability_id: 'workspace.files', action_id: 'workspace.files/read',
    file_summary: { operation: 'read', files: [{ path: tool.content.description! }] } });
  if (tool.content.name === 'apply_patch') Object.assign(tool.content, { capability_id: 'workspace.files', action_id: 'workspace.files/patch',
    file_summary: { operation: 'patch', files: [{ path: tool.content.description!, added_lines: 8, removed_lines: 3 }] } });
}
const publicResult = '已读取两个文件并更新测试。策划与交互规范已返回，接下来核对开发与架构结果。';
tools.push({ id: 'completion-receipt', type: 'tool_call', conversation_id: leadConversationId, turn_id: requestId,
  content: { call_id: 'completion-receipt', name: 'report_completion', description: publicResult, status: 'completed', result_seq: 48 } });
const records = [
  ...steps.map(step => makeDelegationRecord({ id: `start:${step.step_id}`, title: step.title, step_id: step.step_id })),
  ...attempts.map((attempt, index) => makeDelegationRecord({ id: `done:${attempt.attempt_id}`, kind: 'completed',
    title: steps[index].title, step_id: attempt.step_id, attempt_id: attempt.attempt_id, execution_event_sequence: index + 2,
    anchor: { agent_session_id: leadConversationId, event_id: 'completion-anchor', seq: 50 } })),
];
const thinking: IMessageThinking & { processSequence: number } = { id: 'preview-thinking', type: 'thinking',
  conversation_id: leadConversationId, turn_id: requestId, processSequence: 2,
  content: { content: '先明确任务边界，再协调并行工作并核验结果。', status: 'done' } };
const processItems: Array<(IMessageThinking | IMessageToolCall) & { processSequence: number }> =
  [thinking, ...tools.filter(tool => !isTaskControlReceipt(tool)).map((tool, index) => ({ ...tool, processGroupId: 'canonical', processSequence: 20 + index * 4 }))];
const items = compactFileActivityItems(insertDelegationRecords(processItems, records));
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zh } }, interpolation: { escapeValue: false } });

function Preview() {
  return <main style={{ width: '100%', maxWidth: 980, boxSizing: 'border-box', margin: '0 auto', padding: '30px 34px', color: 'var(--color-text-1)', fontSize: 14, lineHeight: '22px' }}>
    <p style={{ margin: '0 0 22px' }}>我会并行完成游戏策划、交互设计、开发和架构，再汇总验证结果。</p>
    <ToolDetailsProvider scope={leadConversationId}><TurnProcessDisclosure
      item={{ id: 'preview-turn', processItems: items, startAt: 1_000, endAt: 223_000, state: 'completed', running: false, defaultCollapsed: false }}
      renderProcessItem={item => item.type === 'delegation_record' ? <DelegationRecordRow records={item.records} /> : <ProcessTraceItem item={item} />}
      getProcessItemKey={item => item.id} getProcessItemState={() => 'completed'} getProcessItemLayoutKind={item => item.type === 'delegation_record' ? 'delegation' : item.type === 'thinking' ? 'thinking' : 'tool'} />
    </ToolDetailsProvider>
    <p style={{ marginTop: 24 }}>{publicResult}</p>
  </main>;
}
document.documentElement.dataset.theme = 'light';
document.body.style.margin = '0';
document.getElementById('root')!.style.cssText = 'height:100vh;display:flex;flex-direction:column';
createRoot(document.getElementById('root')!).render(<MemoryRouter><I18nextProvider i18n={i18n}>
  <ExecutionProvider conversation={leadConversation}><ExecutionContentSwitcher><ConversationProvider value={{ conversation_id: leadConversationId, type: 'nomi', readOnly: true }}><Preview /></ConversationProvider></ExecutionContentSwitcher></ExecutionProvider>
</I18nextProvider></MemoryRouter>);
