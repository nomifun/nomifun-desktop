/** Production composer/command overlay with local fixture transport only.
 * Open at a supported desktop viewport, including 880x600 and narrow panes. */
import React, { useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { createInstance } from 'i18next';
import '@arco-design/web-react/es/_util/react-19-adapter';
import '@arco-design/web-react/dist/css/arco.css';
import 'virtual:uno.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/arco-override.css';
import messages from '../src/renderer/services/i18n/locales/zh-CN/messages.json';
import conversation from '../src/renderer/services/i18n/locales/zh-CN/conversation.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import type { ComposerCommand } from '../src/renderer/components/chat/commands/types';

globalThis.fetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  status: 200, headers: { 'content-type': 'application/json' },
})) as typeof fetch;
class FixtureSocket extends EventTarget {
  static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
  readyState = 1; send() {} close() { this.readyState = 3; }
}
globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
const { default: Composer, ComposerSendButton } = await import('../src/renderer/components/chat/Composer');
const { ThemeProvider } = await import('../src/renderer/hooks/context/ThemeContext');
const { useComposerCommands } = await import('../src/renderer/components/chat/commands/useComposerCommands');
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { messages, conversation, common } } } });
function Preview() {
  const [target, setTarget] = useState('已有会话');
  const [input, setInput] = useState('/');
  const [narrow, setNarrow] = useState(false);
  const [result, setResult] = useState('等待操作');
  const anchor = useRef<HTMLDivElement>(null);
  const invoke = (name: string) => () => { setResult(`${target}：${name}`); };
  const catalog: ComposerCommand[] = target === '终端' ? [
    { id: 'terminal.clear-view', name: 'clear', description: '清除终端显示', execute: invoke('清屏') },
    { id: 'terminal.interrupt', name: 'interrupt', description: '中断当前终端进程', execute: invoke('中断') },
  ] : [
    { id: 'files.open', name: 'open', description: '添加文件', execute: invoke('添加文件') },
    ...(['copy', 'export', 'clear'] as const).map(name => ({ id: `session.${name}`, name,
      description: { copy: '复制回复', export: '导出完整会话', clear: '清空模型上下文，保留记录' }[name],
      ...(target === '新建会话' ? { disabledReason: '发送首条消息后可用' } : { execute: invoke(name) }),
    })),
  ];
  const commands = useComposerCommands({ targetKey: target, input, setInput, commands: catalog, anchorRef: anchor });
  const submit = () => { if (!commands.dispatchInput()) setResult(`普通输入：${input}`); };
  return <main className='command-preview'>
    <header><strong>会话快捷命令 · 桌面验证</strong><p>真实输入框与公共弹层，仅使用测试动作。</p></header>
    <nav>{['新建会话', '已有会话', '终端'].map(name => <button key={name} onClick={() => { setTarget(name); setInput('/'); }}>{name}</button>)}
      <button onClick={() => setNarrow(value => !value)}>切换窄分栏</button></nav>
    <section className='command-preview-workspace'>
      <div className='command-preview-pane' style={{ width: narrow ? 350 : '100%' }}>
        <div style={{ flex: 1 }}>当前：{target}<p>输入 /，或直接输入完整命令后点击发送。</p></div>
        <Composer surfaceRef={anchor} singleLine={false} overlayOpen={commands.isOpen} overlays={commands.menu}
          inputProps={{ value: input, onChange: setInput, placeholder: '输入 / 打开命令菜单',
            onKeyDown: event => { if (!commands.onKeyDown(event) && event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); submit(); } },
          }} actions={<ComposerSendButton disabled={commands.busy} onClick={submit} />} />
      </div>
      {narrow && <aside>模拟桌面右侧工作区</aside>}
    </section>
    <output>{result}</output>
  </main>;
}
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;height:100%;min-width:880px;font-family:sans-serif;background:var(--color-bg-1);color:var(--color-text-1)} .command-preview{height:100%;box-sizing:border-box;padding:20px;display:flex;flex-direction:column;gap:12px}.command-preview header p{margin:4px 0;color:var(--color-text-3)}.command-preview nav{display:flex;gap:8px}.command-preview nav button{padding:6px 12px}.command-preview-workspace{display:flex;flex:1;min-height:0;overflow:hidden;border:1px solid var(--color-border-2);border-radius:12px}.command-preview-pane{padding:14px;box-sizing:border-box;display:flex;flex-direction:column;min-width:0;overflow:hidden}.command-preview aside{padding:14px;flex:1;border-left:1px solid var(--color-border-2)}';
document.head.appendChild(style);
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
root.render(<I18nextProvider i18n={i18n}><ThemeProvider><Preview /></ThemeProvider></I18nextProvider>);
