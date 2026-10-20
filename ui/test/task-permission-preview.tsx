/** Desktop browser harness: production MessageList/settings, in-memory API/native mocks. */
import React, { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter, useLocation, useNavigate } from 'react-router-dom';
import { ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { FolderOpen, Plus, SettingTwo, Search, Terminal, Message, Earth, Refresh, ArrowUp } from '@icon-park/react';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/feedback-bubble-contract.css';
import zh from '../src/renderer/services/i18n/locales/zh-CN';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '../src/renderer/pages/settings/DisplaySettings/presets';
import type { SystemPermissionStatus } from '../src/common/adapter/ipcBridge';
import type { OfficialPresetTemplate } from '../src/common/types/agentPlatform';
import type { GuidAgentSelection } from '../src/renderer/pages/guid/types';
import type { AgentResourceSelectionValue } from '../src/renderer/hooks/agent/agentResourceSelection';
import type { ConversationContextValue } from '../src/renderer/hooks/context/ConversationContext';
import seed from '../../crates/backend/nomifun-agent-contracts/contracts/presets/official-agent-seed-manifest.payload.json';
import { taskPermissionBrowserMessages, taskPermissionCall, taskPermissionColdRootId, taskPermissionConversationId as conversationId,
  taskPermissionMessages, taskPermissionMixedMessages, taskPermissionOrdinaryMessages, taskPermissionUser } from './fixtures/taskPermission';

let granted = false;
let checks = 0;
let requestCount = 0;
const settingsCalls: string[] = [];
const unsupported: string[] = [];
let publishTelemetry: (() => void) | undefined;
const systemStatus = (): SystemPermissionStatus => ({ platform: 'macos', app_label: 'NomiFun', permissions: [
  { kind: 'microphone', state: 'granted', can_request: false, can_open_settings: true, requires_restart_after_grant: false, capabilities: ['voice_input'] },
  { kind: 'accessibility', state: granted ? 'granted' : 'denied', can_request: false, can_open_settings: true, requires_restart_after_grant: false, capabilities: ['computer_use'] },
  { kind: 'screen_recording', state: 'granted', can_request: false, can_open_settings: true, requires_restart_after_grant: false, capabilities: ['computer_use'] },
] });
const preferences: Record<string, unknown> = { 'chat.thinking.visible': true };
const response = (data: unknown, status = 200) => new Response(JSON.stringify({ success: status < 400, data }),
  { status, headers: { 'Content-Type': 'application/json' } });
// No call falls through to a real backend or host setting change.
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const path = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href).pathname;
  if (path === '/api/settings/client') {
    if (init?.method === 'PUT' && typeof init.body === 'string') Object.assign(preferences, JSON.parse(init.body));
    return response(preferences);
  }
  if (path === '/api/providers') return response([]);
  if (path === '/api/browser-providers/attached-chrome' && (!init?.method || init.method === 'GET')) return response(null);
  unsupported.push(path); publishTelemetry?.();
  return response('Preview transport is isolated.', 403);
}) as typeof fetch;
const NativeWebSocket = globalThis.WebSocket;
class PreviewWebSocket extends EventTarget {
  static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
  readyState = PreviewWebSocket.CLOSED;
  onopen = null; onclose = null; onerror = null; onmessage = null;
  constructor(url: string | URL, protocols?: string | string[]) {
    super();
    if (new URL(String(url), location.href).pathname !== '/ws') return new NativeWebSocket(url, protocols) as unknown as PreviewWebSocket;
  }
  send() {} close() {}
}
globalThis.WebSocket = PreviewWebSocket as unknown as typeof WebSocket;

const { ipcBridge } = await import('../src/common');
const { configService } = await import('../src/common/config/configService');
ipcBridge.systemPermissions.get.invoke = async () => { checks += 1; publishTelemetry?.(); return systemStatus(); };
ipcBridge.systemPermissions.request.invoke = async () => { requestCount += 1; publishTelemetry?.(); return systemStatus(); };
ipcBridge.systemPermissions.openSettings.invoke = async ({ kind }) => { settingsCalls.push(kind); publishTelemetry?.(); };
ipcBridge.notification.permissionState.invoke = async () => 'granted';
ipcBridge.notification.requestPermission.invoke = async () => 'granted';
ipcBridge.conversation.turnFiles.invoke = async () => [];
ipcBridge.conversation.turnCompleted.on = () => () => {};
ipcBridge.conversation.reconnected.on = () => () => {};
ipcBridge.fileStream.contentUpdate.on = () => () => {};
ipcBridge.knowledge.onTreeChanged.on = () => () => {};
ipcBridge.knowledge.onEntryContentUpdated.on = () => () => {};
ipcBridge.workspaceOfficeWatch.start.invoke = async () => {};
ipcBridge.workspaceOfficeWatch.stop.invoke = async () => {};
ipcBridge.workspaceOfficeWatch.fileAdded.on = () => () => {};
ipcBridge.fs.listWorkspaceFiles.invoke = async () => [];
ipcBridge.mode.onProvidersChanged.on = () => () => {};
await configService.initialize();
const { default: MessageList } = await import('../src/renderer/pages/conversation/Messages/MessageList');
const { MessageListProvider, useUpdateMessageList } = await import('../src/renderer/pages/conversation/Messages/hooks');
const { ConversationProvider } = await import('../src/renderer/hooks/context/ConversationContext');
const { PreviewProvider } = await import('../src/renderer/pages/conversation/Preview');
const { default: CapabilityPermissionsContent } = await import('../src/renderer/components/settings/SettingsModal/contents/CapabilityPermissionsContent');
const { default: AgentSelector } = await import('../src/renderer/components/agent/AgentSelector');
const { default: AgentResourcePicker } = await import('../src/renderer/components/agent/AgentResourcePicker');
const officialTemplates = Object.entries(seed.templates).map(([key, value]) => ({ template_key: key, seed: value,
  role_coverage: seed.role_coverage[key as keyof typeof seed.role_coverage], immutable: true, forkable: true })) as OfficialPresetTemplate[];
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zh } }, interpolation: { escapeValue: false } });

function Preview() {
  const navigate = useNavigate();
  const route = useLocation();
  const update = useUpdateMessageList();
  const currentTurn = useRef(3);
  const attempt = useRef(1);
  const [, telemetry] = useState(0);
  const [title, setTitle] = useState('日常对话');
  const [theme, setTheme] = useState<'light' | 'dark'>('light');
  const [agentSelection, setAgentSelection] = useState<GuidAgentSelection>({ kind: 'template', templateKey: 'assistant.general' });
  const [resourceSelection, setResourceSelection] = useState<AgentResourceSelectionValue>({});
  const [context, setContext] = useState<ConversationContextValue>({ conversation_id: conversationId,
    type: 'nomi', isProcessing: false, isTurnStateHydrated: true });
  useEffect(() => { publishTelemetry = () => telemetry(value => value + 1); return () => { publishTelemetry = undefined; }; }, []);
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
    document.documentElement.setAttribute('data-color-scheme', 'default');
    document.body.setAttribute('arco-theme', theme);
  }, [theme]);
  const select = (kind: 'normal' | 'computer' | 'incomplete' | 'browser' | 'cold' | 'mixed') => {
    navigate('/conversation');
    granted = false;
    currentTurn.current += 1;
    attempt.current = 1;
    setTitle(kind === 'normal' ? '日常对话' : kind === 'browser' ? '查看浏览器页面' : '整理下载文件夹');
    const rows = kind === 'normal' ? taskPermissionOrdinaryMessages : kind === 'browser' ? taskPermissionBrowserMessages
      : kind === 'mixed' ? taskPermissionMixedMessages(currentTurn.current)
      : taskPermissionMessages(currentTurn.current, kind === 'incomplete' || kind === 'cold');
    const request = taskPermissionUser(currentTurn.current);
    setContext({ conversation_id: conversationId, type: 'nomi', isProcessing: false, isTurnStateHydrated: true,
      ...(kind === 'cold' ? { activeTurnId: taskPermissionColdRootId, activeRequestMessageId: request.msg_id } : {}) });
    update(rows);
    publishTelemetry?.();
  };
  return <>
    <div className='permission-preview-controls' aria-label='验收控制'>
      <span>真实组件 · API 与系统操作为 mock</span>
      <button onClick={() => select('normal')}>普通对话</button>
      <button onClick={() => select('computer')}>Computer 缺权限</button>
      <button onClick={() => select('incomplete')}>失败 Turn 汇总</button>
      <button onClick={() => select('mixed')}>混合受阻</button>
      <button onClick={() => select('browser')}>Browser 已就绪</button>
      <button onClick={() => select('cold')}>冷历史映射</button>
      <button onClick={() => { attempt.current += 1; update(rows => [...rows, taskPermissionCall(currentTurn.current, attempt.current)]); }}>同任务重试</button>
      <button onClick={() => { granted = true; window.dispatchEvent(new Event('focus')); publishTelemetry?.(); }}>模拟授权并返回</button>
      <button onClick={() => { granted = false; currentTurn.current += 1; attempt.current = 1;
        setContext({ conversation_id: conversationId, type: 'nomi', isProcessing: false, isTurnStateHydrated: true });
        update(rows => [...rows, ...taskPermissionMessages(currentTurn.current)]); publishTelemetry?.(); }}>新任务再次缺权限</button>
      <button onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>{theme === 'light' ? '深色' : '浅色'}</button>
    </div>
    <div className='permission-preview-shell'>
      <aside className='permission-preview-sidebar'>
        <div className='permission-preview-sidebar-title'><span>会话</span><SettingTwo size={16} /></div>
        <div className='permission-preview-quick'><span><Plus size={15} />会话</span><span><Terminal size={15} />终端</span><span><Plus size={15} />项目</span><span><Plus size={15} />远程</span></div>
        <p className='permission-preview-search'><Search size={17} />搜索</p>
        <p className='permission-preview-muted'>项目/工作路径</p>
        <p><FolderOpen size={17} />默认工作路径 <span className='permission-preview-muted'>2</span></p>
        <p className='permission-preview-muted'>交互会话（2）</p>
        <div className='permission-preview-history'>帮我生成一只…</div><div className='permission-preview-history'>帮我写一个贪…</div>
      </aside>
      <main className='permission-preview-main'>
        <div className='permission-preview-top'><span>知识库</span><span>自动工作</span><span>智能决策</span></div>
        <h1>{title}</h1>
        <div className='permission-preview-timeline'><ConversationProvider value={context}><MessageList /></ConversationProvider></div>
        <div className='permission-preview-composer'>
          <div className='permission-preview-project'><FolderOpen size={15} />选择项目</div>
          <div className='permission-preview-input'><div className='permission-preview-input-tabs'>
            <AgentSelector presets={[]} officialTemplates={officialTemplates} selection={agentSelection}
              onSelectTemplate={templateKey => setAgentSelection({ kind: 'template', templateKey })}
              onSelectPreset={presetId => setAgentSelection({ kind: 'preset', presetId })}
              resourceConfiguration={<AgentResourcePicker requiredKinds={['computer', 'browser']}
                capabilityIds={['computer', 'browser']} actionIds={['computer/input', 'browser/observe']}
                value={resourceSelection} onChange={setResourceSelection} presentation='embedded'
                loadInventory={async () => ({ options: {}, errors: {} })} />} />
            <span><Message size={16} />日常对话</span></div>
            <textarea aria-label='继续对话，不会发送' placeholder='继续对话…' />
            <div className='permission-preview-input-footer'><Plus size={18} /><span>step-5-preview　⌄　自动　⌄</span><ArrowUp size={18} /></div>
          </div>
        </div>
        <footer className='permission-preview-footer'><span>帮助与反馈</span><span><Earth size={15} />WebUI 运行中 <Refresh size={15} />检查更新</span></footer>
      </main>
      {route.pathname === '/settings/permissions' && <div className='permission-preview-settings' role='dialog' aria-modal='true' aria-label='权限设置预览'>
        <div className='permission-preview-settings-tools'><strong>生产权限设置页</strong><button onClick={() => navigate('/conversation')}>返回任务</button></div>
        <CapabilityPermissionsContent />
      </div>}
    </div>
    <output className='permission-preview-telemetry' aria-label='验收调用记录'>
      {JSON.stringify({ checks, requestCount, settingsCalls, unsupported, granted, route: route.pathname + route.search })}
    </output>
  </>;
}
const style = document.createElement('style');
style.textContent = `html,body,#root{margin:0;height:100%;min-width:880px;min-height:600px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif;color:var(--color-text-1);background:var(--color-bg-1)}*{box-sizing:border-box}.permission-preview-controls{height:44px;display:flex;align-items:center;gap:7px;padding:5px 12px;font-size:10px;border-bottom:1px solid var(--color-border-2);white-space:nowrap}.permission-preview-controls button,.permission-preview-settings-tools button{font:inherit;color:var(--color-text-2);background:var(--color-bg-2);border:1px solid var(--color-border-2);border-radius:5px;padding:4px 6px;cursor:pointer}.permission-preview-controls button:focus-visible{outline:2px solid rgb(var(--primary-6));outline-offset:2px}.permission-preview-shell{position:relative;display:grid;grid-template-columns:208px minmax(0,1fr);height:calc(100% - 69px)}.permission-preview-sidebar{border-right:1px solid var(--color-border-2);background:var(--color-fill-1);padding:20px 18px;font-size:13px}.permission-preview-sidebar-title{display:flex;justify-content:space-between;color:var(--color-text-3);margin-bottom:12px}.permission-preview-quick{display:grid;grid-template-columns:1fr 1fr;gap:6px}.permission-preview-quick span{display:flex;align-items:center;justify-content:center;gap:7px;border:1px solid var(--color-border-2);border-radius:8px;height:31px}.permission-preview-sidebar p{display:flex;gap:9px;align-items:center;margin:20px 0 15px}.permission-preview-sidebar .permission-preview-search{font-size:14px;margin-top:23px}.permission-preview-muted{color:var(--color-text-3);font-size:12px}.permission-preview-history{padding:9px 12px;font-size:12px;color:var(--color-text-2)}.permission-preview-main{display:flex;flex-direction:column;min-height:0;min-width:0;padding:20px 30px 16px}.permission-preview-top{display:flex;justify-content:flex-end;gap:10px;font-size:12px;color:var(--color-text-2)}.permission-preview-top span{border:1px solid var(--color-border-2);border-radius:20px;padding:7px 14px;background:rgb(var(--primary-1))}.permission-preview-main h1{font-size:18px;font-weight:600;margin:22px 0 6px}.permission-preview-timeline{flex:1;min-height:0;min-width:0;display:flex;flex-direction:column}.permission-preview-timeline>.message-list-container{flex:1;min-height:0}.permission-preview-composer{padding-top:9px}.permission-preview-project{display:flex;align-items:center;gap:8px;height:35px;margin:0 10px;padding:0 16px;border-radius:15px 15px 0 0;background:rgb(var(--primary-1));font-size:12px;color:var(--color-text-3)}.permission-preview-input{border:1px solid var(--color-border-3);border-radius:22px;padding:14px 18px;background:var(--color-bg-2)}.permission-preview-input-tabs{display:flex;align-items:center;gap:9px;font-size:13px}.permission-preview-input-tabs>span{margin-left:16px;display:flex;align-items:center;gap:7px;background:rgb(var(--primary-1));color:rgb(var(--primary-6));padding:6px 10px;border-radius:7px}.permission-preview-input textarea{background:transparent;border:0;outline:0;resize:none;font:inherit;font-size:13px;color:var(--color-text-1);width:100%;height:43px;padding:9px 0}.permission-preview-input textarea::placeholder{color:var(--color-text-3)}.permission-preview-input-footer{display:flex;align-items:center;gap:12px;color:var(--color-text-3);font-size:12px}.permission-preview-input-footer>span{margin-left:auto;color:var(--color-text-2);border:1px solid var(--color-border-2);border-radius:20px;padding:7px 12px}.permission-preview-footer{display:flex;justify-content:space-between;gap:18px;font-size:12px;color:var(--color-text-2);padding:17px 3px 0}.permission-preview-footer>span:last-child{display:flex;align-items:center;gap:9px}.permission-preview-telemetry{display:block;position:relative;z-index:5;height:25px;line-height:25px;font:10px monospace;white-space:nowrap;padding:0 12px;overflow:hidden;color:var(--color-text-3);background:var(--color-fill-1);border-top:1px solid var(--color-border-2)}.permission-preview-settings{position:absolute;inset:0 0 0 208px;background:var(--color-bg-1);overflow:auto;z-index:3;padding:14px 24px}.permission-preview-settings-tools{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:20px;font-size:12px}.permission-preview-timeline .message-list{padding-top:14px}`;
style.textContent += '.permission-preview-controls{gap:4px;font-size:9px;overflow-x:auto}';
document.head.appendChild(style);
const themeStyle = document.createElement('style');
themeStyle.textContent = PRESET_THEMES.find(item => item.id === DEFAULT_THEME_ID)?.css ?? '';
document.head.appendChild(themeStyle);
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
root.render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN}><MemoryRouter initialEntries={['/conversation']}>
  <PreviewProvider persistNamespace='task-permission-browser-harness' subscribeGlobalOpen={false}>
    <MessageListProvider initialValue={taskPermissionOrdinaryMessages}><Preview /></MessageListProvider>
  </PreviewProvider>
</MemoryRouter></ConfigProvider></I18nextProvider>);
