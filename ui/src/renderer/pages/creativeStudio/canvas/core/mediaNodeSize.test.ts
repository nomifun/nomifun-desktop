import { expect, test } from 'bun:test';
import { canvasMediaNodeSize } from './mediaNodeSize';
import { canvasCommands, canvasReducer, createInitialCanvasState } from './index';
import { testDocument, testNode } from './testFixtures';

test.each([
  [1600, 900, 340 * 16 / 9, 340],
  [900, 1600, 340, 340 * 16 / 9],
  [1024, 1024, 340, 340],
  [100, 2000, 340, 6800],
])('fits %s × %s media by the short edge without clipping', (width, height, expectedWidth, expectedHeight) => {
  const size = canvasMediaNodeSize({ width, height }, { width: 340, height: 340 });
  expect(size.width).toBeCloseTo(expectedWidth);
  expect(size.height).toBeCloseTo(expectedHeight);
});

test('ignores unresolved or invalid metadata and keeps manually chosen scale', () => {
  const base = { width: 450, height: 300 };
  for (const width of [null, 0, -1, NaN, Infinity]) {
    expect(canvasMediaNodeSize({ width, height: 100 }, base)).toEqual(base);
  }
  expect(canvasMediaNodeSize({ width: 100, height: 200 }, base)).toEqual({ width: 300, height: 600 });
});

test.each(['image', 'video'] as const)('media reconciliation preserves %s geometry scale and placement through undo', (kind) => {
  const source = testNode(kind, 1, { width: 340, height: 240 });
  let state = createInitialCanvasState({ document: testDocument([source]) });
  state = canvasReducer(state, canvasCommands.updateNode({
    ...source, size: { width: 500, height: 500 }, position: { x: 100, y: 200 },
  }));
  const current = state.document.nodes[0]!;
  const result = structuredClone(source);
  result.size = { width: 500, height: 750 };
  result.data.assetId = 'asset';
  state = canvasReducer(state, canvasCommands.reconcileRuntimeNode(result));
  expect(state.document.nodes[0]!.size).toEqual({ width: 500, height: 750 });
  expect(state.document.nodes[0]!.position).toEqual(current.position);
  state = canvasReducer(state, canvasCommands.undo());
  expect(state.document.nodes[0]!.size).toEqual({ width: 240, height: 360 });
  expect(state.document.nodes[0]!.position).toEqual(source.position);
  expect(state.document.nodes[0]!.data).toMatchObject({ assetId: 'asset' });
});
