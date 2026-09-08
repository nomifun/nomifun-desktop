import { useState } from 'react';
import { SWRConfig, unstable_serialize } from 'swr';
import ModelAdvancedEditor from '../src/renderer/pages/settings/components/ModelAdvancedEditor';
import { ThemeProvider } from '../src/renderer/hooks/context/ThemeContext';
import { aliasCapability, aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from './fixtures/modelAliasEditor';

const requestKey = JSON.stringify(['preview', aliasProviderBaseUrl, ['chat']]);
const config = {
  provider: () => new Map(), revalidateOnMount: false,
  fallback: {
    [unstable_serialize(['model-protocol-manifests', requestKey])]: { requestKey, manifests: { chat: aliasProtocolManifest }, errorTasks: [] },
    [`provider-connections:${aliasProviderId}`]: [],
  },
};
const capabilities = [aliasCapability];

/** Actual model editor with isolated preview state; no model service is called. */
export default function ModelAliasEditorPreview() {
  const [displayName, setDisplayName] = useState<string | undefined>('日常助理模型');
  return <SWRConfig value={config}><ThemeProvider>
    <main style={{ maxWidth: 760, margin: '64px auto', padding: 24 }}>
      <h2>模型别名编辑预览</h2>
      <p>当前别名：{displayName || '未设置（显示原始模型 ID）'}</p>
      <ModelAdvancedEditor providerId={aliasProviderId} providerName='测试服务商' preset='preview'
        providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' model='preview-model-id'
        displayName={displayName} capabilities={capabilities}
        onSave={async (patch) => setDisplayName(patch.display_name ?? undefined)} />
    </main>
  </ThemeProvider></SWRConfig>;
}
