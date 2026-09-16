/** Isolated visual harness using production components and fixed test data. */
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { HashRouter } from 'react-router-dom';
import { Button, ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import nomi from '../src/renderer/services/i18n/locales/zh-CN/nomi.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import messagesLocale from '../src/renderer/services/i18n/locales/zh-CN/messages.json';
import sessionList from '../src/renderer/services/i18n/locales/zh-CN/sessionList.json';
import previewImage from '../../docs/images/desktop-companion-window-crop.png?url';

globalThis.fetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'content-type': 'application/json' },
})) as typeof fetch;
const { ipcBridge } = await import('../src/common');
for (const group of [ipcBridge.companion, ipcBridge.conversation, ipcBridge.robot]) {
  for (const event of Object.values(group)) if (event && typeof (event as any).on === 'function') (event as any).on = () => () => {};
}
const companionId = '0190f5fe-7c00-7a00-8000-000000000099' as any;
const conversationId = '0190f5fe-7c00-7a00-8000-000000000098' as any;
const permissions = { vision: true, motion: true, display: true, device_tools: false, proactive_speech: true, continuous_vision: false };
let devices = ['书桌机器人', '客厅机器人'].map((name, index) => ({
  robot_id: `device-${index}`, name, companion_id: companionId, board: 'ESP32-S3', firmware_version: '1.9',
  last_seen: new Date().toISOString(), created_at: new Date().toISOString(), permissions: { ...permissions },
  supported_permissions: ['vision', 'motion', 'display', 'device_tools', 'proactive_speech'],
}));
const statuses = devices.map((device) => ({ robot_id: device.robot_id, companion_id: companionId, phase: 'idle', changed_at: Date.now() }));
const profile = { companion_id: companionId, name: '毛球', character: 'ink', seq: 1, model: { provider_id: companionId, model: '预览模型' },
  status: { model_configured: true, level: 3, mood: 'content' } } as any;
ipcBridge.companion.listCompanions.invoke = async () => [profile];
ipcBridge.companion.getCompanionSession.invoke = async () => ({ conversation_id: conversationId }) as any;
ipcBridge.robot.list.invoke = async () => devices as any;
ipcBridge.robot.statuses.invoke = async () => statuses as any;
ipcBridge.robot.endpoints.invoke = async () => ({ ota_urls: ['http://192.168.1.10:25808/robot/ota'], lan_enabled: true });
ipcBridge.robot.setPermissions.invoke = async ({ robot_id, permissions }) => {
  devices = devices.map((device) => device.robot_id === robot_id ? { ...device, permissions } : device);
  return devices.find((device) => device.robot_id === robot_id) as any;
};
ipcBridge.robot.speak.invoke = async () => ({ accepted: true });
ipcBridge.fs.getFileMetadata.invoke = async () => ({ size: 2048 }) as any;
ipcBridge.fs.getImageBase64.invoke = async () => previewImage;

const { default: CompanionSessionGroup } = await import('../src/renderer/pages/conversation/SessionList/CompanionSessionGroup');
const { default: DevicesControl } = await import('../src/renderer/pages/nomi/companion/CompanionDevicesControl');
const { default: DeviceSettings } = await import('../src/renderer/pages/nomi/workspace/tabs/RemoteTab/RobotConnectSection');
const { default: MessageText } = await import('../src/renderer/pages/conversation/Messages/components/MessageText');
const { MessageListProvider } = await import('../src/renderer/pages/conversation/Messages/hooks');
const { ThemeProvider } = await import('../src/renderer/hooks/context/ThemeContext');
const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { nomi, common, messages: messagesLocale, sessionList } } }, interpolation: { escapeValue: false } });
const messages = [
  { type: 'text', id: 'one', message_id: '0190f5fe-7c00-7a00-8000-000000000001', conversation_id: conversationId, position: 'right', content: { content: '记住，明天下午我们要去公园。' } },
  { type: 'text', id: 'two', message_id: '0190f5fe-7c00-7a00-8000-000000000002', conversation_id: conversationId, position: 'left', content: { content: '记住了。明天下午去公园，出发前我们再看看天气。' } },
  { type: 'text', id: 'three', message_id: '0190f5fe-7c00-7a00-8000-000000000003', conversation_id: conversationId, position: 'right', content: {
    content: '我们刚刚说好什么时候去公园？', interaction: { kind: 'robot', robot_id: 'device-0' },
  } },
  { type: 'text', id: 'four', message_id: '0190f5fe-7c00-7a00-8000-000000000004', conversation_id: conversationId, position: 'left', content: { content: '明天下午。你刚才在桌面上告诉我的，我还记得。' } },
  { type: 'text', id: 'five', message_id: '0190f5fe-7c00-7a00-8000-000000000005', conversation_id: conversationId, position: 'right', content: {
    content: '看看桌面上的伙伴。', interaction: { kind: 'robot', robot_id: 'device-0' },
    observations: [{ question: '桌面上是什么？', answer: '桌面上显示着同一个伙伴。', observed_at: Date.now(), image: { id: 'preview-image', path: '/preview/robot-photo.png' } }],
  } },
] as any;
function Preview() {
  const [tab, setTab] = useState('chat');
  const [current, setCurrent] = useState(profile);
  return <div className='preview-shell'>
    <aside><div className='preview-brand'>NomiFun</div><CompanionSessionGroup activeConversationId={conversationId} expanded /></aside>
    <main><header><strong>毛球</strong><DevicesControl conversationId={conversationId} companion={{ profile: current,
      patchCompanion: async (patch: any) => setCurrent((previous: any) => ({ ...previous, ...patch })) } as any} /></header>
      <nav><Button type={tab === 'chat' ? 'primary' : 'text'} onClick={() => setTab('chat')}>会话</Button><Button type={tab === 'devices' ? 'primary' : 'text'} onClick={() => setTab('devices')}>设备设置</Button></nav>
      <section>{tab === 'chat' ? <MessageListProvider initialValue={messages}><div className='preview-messages'>
        {messages.map((message: any) => <MessageText key={message.id} message={message} hideActions />)}
        <div className='preview-input'>给毛球发消息…</div>
      </div></MessageListProvider> : <DeviceSettings companionId={companionId} companionName='毛球' />}</section>
    </main>
  </div>;
}
document.body.setAttribute('data-theme', 'light');
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;height:100%;min-width:880px;font-family:Inter,"PingFang SC",sans-serif;background:var(--color-bg-1);color:var(--color-text-1)}.preview-note{height:28px;display:flex;align-items:center;justify-content:center;font-size:11px;background:var(--color-fill-2);color:var(--color-text-3)}.preview-shell{height:calc(100% - 28px);display:grid;grid-template-columns:220px minmax(0,1fr)}aside{padding:16px 10px;background:var(--color-fill-1);border-right:1px solid var(--color-border-2)}.preview-brand{font-size:18px;font-weight:700;padding:4px 14px 28px}main{min-width:0;display:flex;flex-direction:column}header{height:60px;flex-shrink:0;display:flex;align-items:center;justify-content:space-between;padding:0 24px;border-bottom:1px solid var(--color-border-2)}nav{padding:12px 24px;display:flex;gap:8px}section{padding:0 24px 20px;overflow:auto;min-height:0;flex:1}.preview-messages{max-width:820px;margin:auto;display:flex;flex-direction:column;gap:28px;padding-top:24px}.preview-input{border:1px solid var(--color-border-2);border-radius:12px;padding:20px;color:var(--color-text-3);margin-top:32px}';
document.head.appendChild(style);
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
root.render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN}><ThemeProvider><HashRouter>
  <div className='preview-note'>交互验证 · 仅使用测试数据，不连接后台或模型</div><Preview />
</HashRouter></ThemeProvider></ConfigProvider></I18nextProvider>);
