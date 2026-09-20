/** Browser regression harness: production picker, isolated from user data. */
import React, { useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import agentSettings from '../src/renderer/services/i18n/locales/zh-CN/agentSettings.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import nomi from '../src/renderer/services/i18n/locales/zh-CN/nomi.json';
import type { AgentResourceSelectionValue } from '../src/renderer/hooks/agent/agentResourceSelection';
import type { AgentResourceInventoryLoader } from '../src/renderer/components/agent/AgentResourcePicker';
import guidStyles from '../src/renderer/pages/guid/index.module.css';

const { ipcBridge } = await import('../src/common');
ipcBridge.companion.onCompanionCreated.on = () => () => {};
ipcBridge.companion.onCompanionDeleted.on = () => () => {};
ipcBridge.systemPermissions.get.invoke = async () => ({
  platform: 'windows', app_label: 'NomiFun', permissions: [
    { kind: 'microphone', state: 'unknown', can_request: false, can_open_settings: false, requires_restart_after_grant: false, capabilities: ['voice_input'] },
    { kind: 'accessibility', state: 'not_required', can_request: false, can_open_settings: false, requires_restart_after_grant: false, capabilities: ['computer_use'] },
    { kind: 'screen_recording', state: 'not_required', can_request: false, can_open_settings: false, requires_restart_after_grant: false, capabilities: ['computer_use'] },
  ],
});
const { default: AgentResourcePicker } = await import('../src/renderer/components/agent/AgentResourcePicker');
const loadInventory: AgentResourceInventoryLoader = async () => ({ options: {
  companion: [{ value: 'companion-1', label: '团团', description: '#1', avatar: { character: 'mochi' } }, { value: 'companion-2', label: '毛球', description: '#2', avatar: { character: 'ink' } }],
  channel: [{ value: 'channel-1', label: '测试渠道', ownerDomain: 'companion' }],
  robot: [{ value: 'robot-1', label: '测试机器人', selectable: true, robotPhase: 'idle',
    robotRequiredPermissions: ['vision'], robotDisabledPermissions: [], robotUnsupportedPermissions: [] }],
}, errors: {} });
const i18n = createInstance();
await i18n.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { agentSettings, common, nomi } } }, interpolation: { escapeValue: false } });
function Preview() {
  const container = useRef<HTMLDivElement>(null);
  const [value, setValue] = useState<AgentResourceSelectionValue>({});
  return <ConfigProvider getPopupContainer={() => container.current || document.body}>
    <div className={guidStyles.guidContainer} ref={container}>
      <main className={guidStyles.guidPrimaryStage}>
        <div className={guidStyles.guidLayout}>
          <h1>设备与伙伴资源交互验证</h1><p>生产组件与会话页弹层容器，仅使用测试数据。</p>
          <AgentResourcePicker requiredKinds={['companion', 'companion_memory', 'channel', 'robot']}
            companionBindings capabilityIds={['companion', 'companion.memory', 'channel.messaging', 'robot']}
            actionIds={['companion/evolve', 'companion/learn', 'companion.memory/recall', 'companion.memory/write', 'channel.messaging/reply', 'robot/vision']}
            value={value} onChange={setValue} loadInventory={loadInventory} />
          <pre aria-label="当前选择">{JSON.stringify(value)}</pre>
          <h2>Windows 电脑权限状态</h2>
          <AgentResourcePicker requiredKinds={['computer']} capabilityIds={['computer']}
            actionIds={['computer/observe', 'computer/a11y.observe']} value={{}}
            onChange={() => undefined} />
        </div>
      </main>
    </div>
  </ConfigProvider>;
}
document.body.setAttribute('data-theme', 'light');
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;height:100%;min-width:880px;font-family:sans-serif;background:var(--color-bg-1);color:var(--color-text-1)}';
document.head.appendChild(style);
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
root.render(<I18nextProvider i18n={i18n}><MemoryRouter><Preview /></MemoryRouter></I18nextProvider>);
