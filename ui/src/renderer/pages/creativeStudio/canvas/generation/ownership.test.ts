import { describe, expect, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import { prepareCanvasImageRun } from './plans';
import { canvasResumeRequests } from './recovery';
import { GenerationError } from '@renderer/creation/generationError';

const id = (suffix: number) => `0190f5fe-7c00-7a00-8000-${String(suffix).padStart(12, '0')}`;
const provider: IProvider = {
  id: id(1) as ProviderId, name: 'Images', enabled: true, platform: 'custom', base_url: 'https://example.invalid', auth_scheme: 'bearer', has_credentials: true,
  models: [{ provider_id: id(1) as ProviderId, model: 'image-v1', enabled: true, sort_order: 0, created_at: 1, updated_at: 1,
    capabilities: [{ task: 'image_generation', traits: [], protocol: 'openai.images', connection_role: 'default', allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 }],
  }],
};
const input = {
  catalog: { status: 'ready' as const, providers: [provider], error: null },
  model: { providerId: id(1), model: 'image-v1' }, references: { bindings: [], assets: [] },
  operation: { task: 'image_generation' as const, capability: 't2i' as const },
  prompt: 'Aurora', interfaceMode: 'images' as const, quality: 'auto' as const, width: 1024, height: 1024, aspectRatio: '1:1', count: 1,
};
describe('canvas generation ownership boundary', () => {
  test('requires an explicit matching canvas node instead of synthesizing a standalone owner', () => {
    expect(() => prepareCanvasImageRun(input)).toThrow(GenerationError);
    expect(() => prepareCanvasImageRun({ ...input, canvasId: id(2), owner: { kind: 'conversation_turn', conversationId: id(2), messageId: id(3) } })).toThrow(GenerationError);
    expect(() => prepareCanvasImageRun({ ...input, canvasId: id(2), owner: { kind: 'canvas_node', canvasId: id(4), nodeId: id(3) } })).toThrow(GenerationError);
    const plan = prepareCanvasImageRun({ ...input, canvasId: id(2), nodeId: id(3) });
    expect(plan.input.owner).toEqual({ kind: 'canvas_node', canvasId: id(2), nodeId: id(3) });
  });
  test('does not recover conversation tasks into the canvas runtime', () => {
    expect(canvasResumeRequests([{ taskId: id(4), owner: { kind: 'conversation_turn', conversationId: id(2), messageId: id(3) }, providerId: id(1), model: 'image-v1', task: 'image_generation', capability: 't2i' }])).toEqual([]);
  });
});
