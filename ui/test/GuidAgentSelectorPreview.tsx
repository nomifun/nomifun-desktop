import { useState } from 'react';
import type { OfficialPresetTemplate } from '../src/common/types/agentPlatform';
import type { GuidAgentSelection } from '../src/renderer/pages/guid/types';
import GuidAgentSelector from '../src/renderer/pages/guid/components/GuidAgentSelector';

/** Uses the production selector, with no conversation or model transport. */
export default function GuidAgentSelectorPreview({ templates }: { templates: OfficialPresetTemplate[] }) {
  const [selection, setSelection] = useState<GuidAgentSelection>({ kind: 'default' });
  const [input, setInput] = useState('帮我整理今天的工作计划');
  return <main style={{ maxWidth: 760, margin: '64px auto', padding: 24 }}>
    <div style={{ border: '1px solid var(--color-border-2)', borderRadius: 20, padding: 24, background: 'var(--color-bg-2)' }}>
      <GuidAgentSelector presets={[]} officialTemplates={templates} selection={selection}
        onSelectDefault={() => setSelection({ kind: 'default' })}
        onSelectPreset={(presetId) => setSelection({ kind: 'preset', presetId })}
        onSelectTemplate={(templateKey) => setSelection({ kind: 'template', templateKey })} />
      <textarea aria-label='待发送消息' value={input} onChange={(event) => setInput(event.target.value)}
        style={{ display: 'block', width: '100%', border: 0, resize: 'none', background: 'transparent', color: 'var(--color-text-1)', marginTop: 24, minHeight: 160, outline: 0, font: 'inherit' }} />
    </div>
    <p style={{ color: 'var(--color-text-3)', fontSize: 12 }}>选择器交互预览 · 选择官方 Agent 不跳转、不创建配置；本页不发送消息。</p>
  </main>;
}
