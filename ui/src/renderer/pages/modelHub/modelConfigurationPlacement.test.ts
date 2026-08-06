import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const readSource = (relativePath: string) =>
  readFileSync(new URL(relativePath, import.meta.url), 'utf8');

describe('model-owned tool configuration placement', () => {
  test('execution engines are not part of model management', () => {
    const hubSource = readSource('./index.tsx');

    expect(hubSource.includes("key: 'agents'")).toBe(false);
    expect(hubSource.includes('AgentModalContent')).toBe(false);
    expect(hubSource.includes("searchParams.get('section') === 'agents'")).toBe(true);
    expect(hubSource.includes("'/settings/execution-engines?tab=remote'")).toBe(true);
  });

  test('the MCP page contains only MCP server management', () => {
    const source = readSource(
      '../../components/settings/SettingsModal/contents/ToolsModalContent.tsx'
    );

    expect(source.includes('SpeechToTextSettingsSection')).toBe(false);
    expect(source.includes('settings.imageGeneration')).toBe(false);
    expect(source.includes('tools.speechToText')).toBe(false);
    expect(source.includes('tools.imageGenerationModel')).toBe(false);
    expect(source.includes('ModalMcpManagementSection')).toBe(true);
    expect(source.includes('const visibleMcpServers = useMemo(() => mcpServers, [mcpServers])')).toBe(true);
  });

  test('speech has a dedicated peer section and old copied cards are removed', () => {
    const hubSource = readSource('./index.tsx');
    const speechHost = readSource('./SpeechModelsContent.tsx');
    const speechSource = readSource('./SpeechToTextContent.tsx');
    const creationSource = readSource('./CreationModelsContent.tsx');
    const providerSource = readSource(
      '../../components/settings/SettingsModal/contents/ModelModalContent.tsx'
    );

    expect(hubSource.includes("key: 'speech'")).toBe(true);
    expect(hubSource.includes('<SpeechModelsContent />')).toBe(true);
    // The 语音 section hosts ASR, TTS and the local VAD entry.
    expect(speechHost.includes('<SpeechToTextContent />')).toBe(true);
    // Candidates come from the authoritative catalog resolve, not provider
    // rows + name guessing — reached through the ONE shared selector, which is
    // what performs the `speech_recognition` resolution.
    expect(speechSource.includes("task='speech_recognition'")).toBe(true);
    expect(speechSource.includes('<TaskModelSelect')).toBe(true);
    expect(speechSource.includes('inferCloudSpeechService')).toBe(false);
    expect(creationSource.includes('ImageGenerationToolSettings')).toBe(false);
    expect(providerSource.includes('SpeechToTextCloudSettings')).toBe(false);
  });

  test('MCP diagnostics link to the dedicated MCP page', () => {
    const source = readSource('../../components/media/FileAttachButton.tsx');

    expect(source.includes("navigate('/mcp')")).toBe(true);
    expect(source.includes('/settings/capabilities?tab=tools')).toBe(false);
  });
});
