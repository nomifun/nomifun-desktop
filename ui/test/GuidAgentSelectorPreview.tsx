import { useState } from 'react';
import type { OfficialPresetTemplate } from '../src/common/types/agentPlatform';
import type { GuidAgentSelection } from '../src/renderer/pages/guid/types';
import GuidAgentSelector from '../src/renderer/pages/guid/components/GuidAgentSelector';
import GuidModelSelector from '../src/renderer/pages/guid/components/GuidModelSelector';
import { SWRConfig } from 'swr';
import type { IProvider, TProviderWithModel } from '../src/common/config/storage';
import type { ExecutableAgentPreset } from '../src/renderer/pages/guid/types';

const provider = {
  id: '0190f5fe-7c00-7a00-8000-000000000111', platform: 'preview', name: '测试模型服务',
  base_url: 'https://example.invalid', enabled: true, has_credentials: false, auth_scheme: 'bearer',
  models: ['模型 A', '模型 B'].map((model) => ({ model, display_name: model === '模型 A' ? '日常助理模型' : undefined, enabled: true,
    capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default' }] })),
} as IProvider;
const modelCache = { provider: () => new Map(), revalidateOnMount: false, fallback: { providers: [provider] } };
const personal = {
  preset_id: '0190f5fe-7c00-7a00-8000-000000000112', source: 'user', display_name: '我的研究助理', bound_target_count: 0,
  current_stable_revision: { preset_id: '0190f5fe-7c00-7a00-8000-000000000112', revision: 1, revision_digest: 'a'.repeat(64) },
} as ExecutableAgentPreset;

/** Uses the production selector, with no conversation or model transport. */
export default function GuidAgentSelectorPreview({ templates }: { templates: OfficialPresetTemplate[] }) {
  const [selection, setSelection] = useState<GuidAgentSelection>({ kind: 'default' });
  const [input, setInput] = useState('帮我整理今天的工作计划');
  const [model, setModel] = useState<TProviderWithModel>({ ...provider, use_model: '模型 A' });
  return <SWRConfig value={modelCache}><main style={{ maxWidth: 760, margin: '64px auto', padding: 24 }}>
    <div style={{ border: '1px solid var(--color-border-2)', borderRadius: 20, padding: 24, background: 'var(--color-bg-2)' }}>
      <GuidAgentSelector presets={[personal]} officialTemplates={templates} selection={selection}
        onSelectDefault={() => setSelection({ kind: 'default' })}
        onSelectPreset={(presetId) => setSelection({ kind: 'preset', presetId })}
        onSelectTemplate={(templateKey) => setSelection({ kind: 'template', templateKey })} />
      <textarea aria-label='待发送消息' value={input} onChange={(event) => setInput(event.target.value)}
        style={{ display: 'block', width: '100%', border: 0, resize: 'none', background: 'transparent', color: 'var(--color-text-1)', marginTop: 24, minHeight: 160, outline: 0, font: 'inherit' }} />
      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        <GuidModelSelector isProviderModelMode modelList={[provider]} current_model={model} setCurrentModel={async (next) => setModel(next)} />
      </div>
    </div>
    <p style={{ color: 'var(--color-text-3)', fontSize: 12 }}>选择器交互预览 · 可切换官方、个人 Agent 和模型；本页不发送消息。</p>
  </main></SWRConfig>;
}
