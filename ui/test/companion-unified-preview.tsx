/** Production companion controls, local fixture APIs; no user data or device access. */
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { Button, ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { SWRConfig } from 'swr';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import nomi from '../src/renderer/services/i18n/locales/zh-CN/nomi.json';
import agentSettings from '../src/renderer/services/i18n/locales/zh-CN/agentSettings.json';
import conversationLabels from '../src/renderer/services/i18n/locales/zh-CN/conversation.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import settings from '../src/renderer/services/i18n/locales/zh-CN/settings.json';
import type { ICompanionProfile, ICompanionStatus, ICompanionThread, IAgentSessionObservation, IApiRobot } from '../src/common/adapter/ipcBridge';
import type { TChatConversation } from '../src/common/config/storage';
import { parseCompanionId, parseConversationId } from '../src/common/types/ids';
import type { CompanionHandle } from '../src/renderer/pages/nomi/workspace/types';
import { scenarioProvider } from './fixtures/scenarioModelEditing';
import styles from '../src/renderer/pages/nomi/workspace/CompanionCohabitView.module.css';

const { ipcBridge } = await import('../src/common');
const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000001');
const windowId = parseConversationId('019b0000-0000-7000-8000-000000000002');
const deviceId = parseConversationId('019b0000-0000-7000-8000-000000000003');
const provider = { ...scenarioProvider, models: [
  { ...scenarioProvider.models[0], model: 'model-a', display_name: '日常模型', enabled: true },
  { ...scenarioProvider.models[0], model: 'model-b', display_name: '工作模型', enabled: true },
] };
const params = new URLSearchParams(location.search);
const mode = params.get('theme') === 'dark' ? 'dark' : 'light';
document.body.setAttribute('data-theme', mode);
if (mode === 'dark') document.body.setAttribute('arco-theme', 'dark');
const noEvent = () => () => {};
ipcBridge.mode.onProvidersChanged.on = noEvent;
ipcBridge.conversation.reconnected.on = noEvent;
ipcBridge.robot.onStatus.on = noEvent;
ipcBridge.mode.listProviders.invoke = async () => [provider];
ipcBridge.mcpService.listServers.invoke = async () => [{ mcp_server_id: 'fixture-mcp', name: '测试办公 MCP', enabled: true }] as never;
const selection = { kind: 'template' as const, template_key: 'companion.default' as const };
ipcBridge.agentPlatform.productBindingOptions.invoke = async () => ({ selection, needs_model: false, options: [
  { selection, available: true, display_name: '', reason: null },
  { selection: { kind: 'preset', preset_id: 'fixture-personal' }, display_name: '我的伙伴配置', available: true, reason: null },
] });
ipcBridge.agentPlatform.selectProductBinding.invoke = async ({ request }) => ({ selection: request.selection, needs_model: false });
ipcBridge.conversation.switchModel.invoke = async () => true;
const binding = { binding_version: 1, typed_resource_bindings: [] };
ipcBridge.agentPlatform.sessions.get.invoke = async () => ({ session: { agent_binding: binding } }) as unknown as IAgentSessionObservation;
ipcBridge.robot.list.invoke = async () => [{ robot_id: 'fixture-device', companion_id: companionId, name: '桌上机器狗（离线）', capability_revision: 0 }] as IApiRobot[];
const { default: CompanionSessionToolbar } = await import('../src/renderer/pages/nomi/workspace/CompanionSessionToolbar');
const { default: CompanionAgentModelQuickSettings } = await import('../src/renderer/pages/nomi/workspace/CompanionAgentModelQuickSettings');
const { default: CompanionRobotResourceSettings } = await import('../src/renderer/pages/nomi/workspace/CompanionRobotResourceSettings');
const { default: ModelsSection } = await import('../src/renderer/pages/nomi/workspace/tabs/OverviewTab/ModelsSection');
const { default: CompanionMcpSettings } = await import('../src/renderer/pages/nomi/workspace/tabs/RemoteTab/CompanionMcpSettings');
const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { nomi, agentSettings, conversation: conversationLabels, common, settings } } }, interpolation: { escapeValue: false } });
function Preview() {
  const [defaults, setDefaults] = useState({ companion_id: companionId, name: '团团', model: { provider_id: provider.id, model: 'model-a' }, mcp_servers: [],
    voice: { vad: { sensitivity: 0.5, min_silence_ms: 500 }, asr: null, tts: null }, fallback_model: null, vision_model: null,
  } as unknown as ICompanionProfile);
  const [view, setView] = useState<'session' | 'defaults'>('session');
  const [selected, setSelected] = useState(params.get('entry') === 'device' ? deviceId : windowId);
  const [receiver, setReceiver] = useState(windowId);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [items, setItems] = useState<ICompanionThread[]>([
    { companion_id: companionId, conversation_id: windowId, role: 'companion', title: '桌面工作', archived: false, created_at: 1, updated_at: 1 },
    { companion_id: companionId, conversation_id: deviceId, role: 'robot', robot_id: 'fixture-device', title: '机器狗对话', archived: false, created_at: 2, updated_at: 2 },
  ]);
  const item = items.find(row => row.conversation_id === selected)!;
  const conversation = { id: selected, name: item.title, type: 'nomi', model: { ...provider, use_model: 'model-a' }, status: 'pending',
    extra: { workspace: '', companion_id: companionId, companion_role: item.role }, runtime: { state: 'idle', is_processing: false, can_send_message: true },
    agent_snapshot: { enabled_capabilities: ['robot'], canonical_binding: binding },
  } as unknown as TChatConversation;
  const companion = { profile: defaults, patchCompanion: async patch => { const next = { ...defaults, ...patch } as ICompanionProfile; setDefaults(next); return next; } } as CompanionHandle;
  return <div className='fixture-root'><header className='fixture-header'><strong>统一伙伴 · Renderer 夹具</strong><span>测试 API，无供应商／设备访问</span>
    <Button size='small' onClick={() => setView(view === 'session' ? 'defaults' : 'session')}>{view === 'session' ? '后续默认配置' : '返回独立会话'}</Button></header>
    {view === 'defaults' ? <main className='fixture-defaults'><ModelsSection companion={companion} status={{ model_configured: true } as ICompanionStatus} companionName='团团' /><CompanionMcpSettings companion={companion} /></main>
    : <div className={styles.layout}>
      <section className={styles.chatColumn}><CompanionSessionToolbar items={items} selectedId={selected} selectedName={item.title} selectedLoaded entryKind={item.role}
        activeDesktopId={receiver} creating={false} routing={false} managing={false} canCreate failed={false} onSelect={setSelected} onCreate={() => {}} onContinue={() => setReceiver(selected)} onRefresh={() => {}}
        onRename={async (id, title) => setItems(items.map(row => row.conversation_id === id ? { ...row, title } : row))} onArchive={async () => {}} onDelete={async () => {}} onClear={async () => {}} />
        <main className='fixture-chat'><h2>团团</h2><p>窗口与设备使用同一个伙伴 Agent，各自保留独立会话。</p><p>当前会话：{item.title}</p><p>后台工作可继续运行；查看其他会话不会更改浮窗接收目标。</p><div className='fixture-note'>聊天正文区域为夹具；上方会话选择器及右侧配置均为生产组件。</div></main>
        <textarea aria-label='当前会话草稿' placeholder='输入独立会话草稿' value={drafts[selected] ?? ''} onChange={event => setDrafts({ ...drafts, [selected]: event.target.value })} />
      </section>
      <aside className={styles.info}><h2>伙伴信息</h2><section className={styles.infoGroup}><div className={styles.infoTitle}>伙伴 Agent</div><CompanionAgentModelQuickSettings companion={companion} conversation={conversation} /></section>
        <section className={styles.infoGroup}><div className={styles.infoTitle}>连接</div><strong>1 台机器人已绑定</strong><CompanionRobotResourceSettings companionId={companionId} conversation={conversation} /></section>
        <p className='fixture-receiver'>浮窗接收：{receiver === windowId ? '桌面工作' : '机器狗对话'}</p>
      </aside>
    </div>}
  </div>;
}
const css = document.createElement('style');
css.textContent = 'html,body,#root{margin:0;height:100%;min-width:880px;font:14px sans-serif;background:var(--color-bg-1);color:var(--color-text-1)}*{box-sizing:border-box}.fixture-root{height:100%;display:flex;flex-direction:column}.fixture-header{display:flex;align-items:center;gap:14px;padding:10px 14px;border-bottom:1px solid var(--color-border-2)}.fixture-header span{flex:1;font-size:12px;color:var(--color-text-3)}.fixture-chat{flex:1;padding:30px;overflow:auto}.fixture-note{margin-top:40px;font-size:12px;color:var(--color-text-3)}textarea{margin:12px;min-height:85px;resize:vertical;background:var(--color-bg-1);color:var(--color-text-1);border:1px solid var(--color-border-2);border-radius:10px;padding:12px}.fixture-receiver{font-size:12px;padding:10px}.fixture-defaults{overflow:auto;padding:16px 28px;flex:1}.fixture-defaults>section{max-width:1000px;margin:auto}';
document.head.append(css);
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
root.render(<ConfigProvider locale={zhCN}><SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><MemoryRouter><I18nextProvider i18n={i18n}><Preview /></I18nextProvider></MemoryRouter></SWRConfig></ConfigProvider>);
