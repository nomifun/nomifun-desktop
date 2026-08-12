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

  test('every model capability is its own section; no host page hides categories', () => {
    const hubSource = readSource('./index.tsx');
    const asrSource = readSource('./SpeechToTextContent.tsx');
    const imageSource = readSource('./ImageModelsContent.tsx');
    const imageEditSource = readSource('./ImageEditModelsContent.tsx');
    const providerSource = readSource(
      '../../components/settings/SettingsModal/contents/ModelModalContent.tsx'
    );

    // 语音 and 创作能力 were hosts stacking several categories behind one entry.
    expect(hubSource.includes("key: 'asr'")).toBe(true);
    expect(hubSource.includes("key: 'tts'")).toBe(true);
    expect(hubSource.includes("key: 'image'")).toBe(true);
    expect(hubSource.includes("key: 'image-edit'")).toBe(true);
    expect(hubSource.includes("key: 'video'")).toBe(true);
    expect(hubSource.includes("key: 'embedding'")).toBe(true);
    expect(hubSource.includes("key: 'rerank'")).toBe(true);
    expect(hubSource.includes('SpeechModelsContent')).toBe(false);
    expect(hubSource.includes('<SpeechToTextContent />')).toBe(true);
    expect(hubSource.includes('<TextToSpeechContent />')).toBe(true);

    // VAD is not a model picker (bundled local Silero), so it rides along with
    // recognition — it decides when listening starts and stops.
    expect(asrSource.includes("t('settings.modelHub.speech.vadTitle')")).toBe(true);

    // Candidates come from the authoritative catalog resolve, not provider
    // rows + name guessing — reached through the ONE shared selector, which is
    // what performs the `speech_recognition` resolution.
    expect(asrSource.includes("task='speech_recognition'")).toBe(true);
    expect(asrSource.includes('<TaskModelSelect')).toBe(true);
    expect(asrSource.includes('inferCloudSpeechService')).toBe(false);

    // Image generation and image edit have distinct task projections.
    expect(imageSource.includes("modality='image'")).toBe(true);
    expect(imageEditSource.includes("modality='image_edit'")).toBe(true);
    expect(providerSource.includes('SpeechToTextCloudSettings')).toBe(false);
  });

  test('MCP diagnostics link to the dedicated MCP page', () => {
    const source = readSource('../../components/media/FileAttachButton.tsx');

    expect(source.includes("navigate('/mcp')")).toBe(true);
    expect(source.includes('/settings/capabilities?tab=tools')).toBe(false);
  });
});
