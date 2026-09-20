import { expect, test } from 'bun:test';
import type { TFunction } from 'i18next';
import { testNode } from '../core/testFixtures';
import { canvasNodeDisplayNames } from './nodeDisplayNames';
import { createCreativeCanvasProductNode, creativeNodeFromAsset } from './nodeFactory';
import { createInitialCanvasState } from '../core';
import type { CreativeAsset } from '../../assets';

const t = ((key: string) => ({ text: '文本', image: '图片', video: '视频', audio: '音频', panorama: '全景', group: '节点组' })[key.split('.').at(-1)! as 'text']) as TFunction;
test('numbers node kinds independently and uses a real filename when available', () => {
  const text = testNode('text', 1), image = testNode('image', 2), second = testNode('image', 3);
  image.data.assetId = 'asset';
  image.zIndex = 999;
  const nodes = [text, image, second, testNode('text', 4), testNode('video', 5)];
  const names = canvasNodeDisplayNames(nodes, new Map([['asset', { title: 'portrait.png' } as CreativeAsset]]), t);
  expect([...names.values()]).toEqual(['文本1', 'portrait.png', '图片2', '文本2', '视频1']);
});
test('all empty user nodes start square; imported image and video nodes fit their aspect ratio', () => {
  const state = createInitialCanvasState();
  for (const kind of ['text', 'image', 'video', 'audio', 'panorama', 'group'] as const) {
    expect(createCreativeCanvasProductNode(kind, state, { width: 1000, height: 800 }).size)
      .toEqual({ width: 288, height: 288 });
  }
  for (const kind of ['image', 'video'] as const) {
    const asset = { id: 'asset', kind, title: 'portrait', width: 600, height: 900 } as CreativeAsset;
    const node = creativeNodeFromAsset(asset, state, { width: 1000, height: 800 });
    expect(node.size).toEqual({ width: 320, height: 480 });
  }
});
