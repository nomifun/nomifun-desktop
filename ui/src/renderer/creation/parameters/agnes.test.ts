import { describe, expect, test } from 'bun:test';
import { agnesImageSizePolicy, agnesVideoSizeOptions, agnesCanvasVideoParameters, agnesVideoPolicy } from './agnes';
import { prepareCanvasVideoRun } from '@renderer/pages/creativeStudio/canvas/generation/plans';
import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import { normalizeCreationParameters } from '../parameterPolicy';

describe('Agnes provider parameter policies', () => {
  test('current image options preserve official ratios and native tier dimensions', () => {
    for (const model of ['agnes-image-2.1-flash', 'agnes-image-2.5-flash']) {
      const policy = agnesImageSizePolicy({ model, protocol: 'agnes.images' })!;
      expect(policy.options.find(value => value.aspectRatio === '16:9' && value.resolution === '2K'))
        .toMatchObject({ width: 2624, height: 1472, requestSize: '2624x1472' });
      expect(policy.options.find(value => value.aspectRatio === '9:16' && value.resolution === '4K'))
        .toMatchObject({ width: 2944, height: 5248 });
      expect(policy.allowCustomDimensions).toBe(false);
    }
    expect(agnesImageSizePolicy({ model: 'agnes-image-2.0-flash', protocol: 'agnes.images' })?.options.some(value => value.requestSize === '1024x768')).toBe(true);
    expect(agnesImageSizePolicy({ model: 'gpt-image-1', protocol: 'openai.images' })).toBeNull();
  });

  test('Flash cannot advertise HD sizes and retired video has no suggested parameters', () => {
    const flash = { model: 'agnes-video-2.5-flash', protocol: 'agnes.video_jobs' };
    expect(agnesVideoSizeOptions(flash).filter(value => value.resolution).every(value => value.resolution === '720P')).toBe(true);
    expect(agnesVideoSizeOptions(flash).find(value => value.aspectRatio === '1:1')).toMatchObject({ width: 720, height: 720 });
    expect(agnesVideoSizeOptions(flash).find(value => value.aspectRatio === '16:9')).toMatchObject({ width: 1280, height: 704 });
    expect(agnesVideoSizeOptions(flash).filter(value => value.resolution).map(value => value.requestSize))
      .toEqual(['1680x720', '1280x704', '960x720', '720x720', '720x960', '720x1280']);
    expect(agnesVideoPolicy(flash).seconds).not.toContain(15);
    const standard = agnesVideoSizeOptions({ ...flash, model: 'agnes-video-2.5' });
    expect(standard.find(value => value.aspectRatio === '1:1' && value.resolution === '720P')).toMatchObject({ width: 960, height: 960 });
    expect(standard.some(value => value.requestSize === '1024x1024' && value.resolution === '1K')).toBe(true);
    expect(standard.some(value => value.requestSize === '2560x1440' && value.resolution === '2K')).toBe(true);
    expect(agnesVideoPolicy({ ...flash, model: 'agnes-video-v2.0' })).toEqual({ seconds: [], sizes: [] });
    expect(agnesCanvasVideoParameters({ model: 'other', protocol: 'ark.video_jobs' }, '1080p', '9:16')).toEqual({});
  });

  test('canvas admission preserves Agnes native aspect ratio through the shared request vocabulary', () => {
    const providerId = '019b0000-0000-7000-8000-000000000009' as ProviderId;
    const provider = { id: providerId, name: 'Agnes', platform: 'agnes', enabled: true, models: [{
      model: 'agnes-video-2.5', enabled: true, capabilities: [{ task: 'video_generation', protocol: 'agnes.video_jobs', traits: [] }],
    }] } as unknown as IProvider;
    const run = prepareCanvasVideoRun({
      catalog: { status: 'ready', providers: [provider], error: null },
      canvasId: '019b0000-0000-7000-8000-000000000001', nodeId: '019b0000-0000-7000-8000-000000000002',
      model: { providerId, model: 'agnes-video-2.5' }, references: { bindings: [], assets: [] },
      operation: { task: 'video_generation', capability: 't2v' }, prompt: 'waves',
      seconds: 5, resolution: '1080p', aspectRatio: '9:16', width: null, height: null, taskCount: 1,
    });
    expect(run.input.parameters).toMatchObject({ size: '1080P', aspect_ratio: '9:16', seconds: 5 });
  });

  test('saved Flash 720P choices retain their ratio using current dimensions, without changing other models', () => {
    const flash = { model: 'agnes-video-2.5-flash', protocol: 'agnes.video_jobs' };
    for (const [previous, current] of [
      ['1470x630', '1680x720'], ['1280x720', '1280x704'], ['1112x834', '960x720'],
      ['960x960', '720x720'], ['834x1112', '720x960'], ['720x1280', '720x1280'],
    ]) {
      expect(normalizeCreationParameters('video', { size: previous, seconds: 4 }, flash).size).toBe(current);
    }
    expect(normalizeCreationParameters('video', { size: '1920x1080' }, flash).size).toBeUndefined();
    expect(normalizeCreationParameters('video', { size: '960x960' }, { ...flash, model: 'agnes-video-2.5' }).size).toBe('960x960');
    expect(normalizeCreationParameters('video', { size: '1280x720' }, { model: 'sora-2', protocol: 'openai.videos' }).size).toBe('1280x720');
  });
});
