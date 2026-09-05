/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset } from '../../assets';
import { createInitialCanvasState } from '../core';
import { testDocument, testEdge, testNode, testUuid } from '../core/testFixtures';
import {
  compileCanvasImageReferencePrompt,
  evaluateCanvasImageGenerationGate,
  MAX_CANVAS_IMAGE_REFERENCE_BYTES,
  resolveCanvasImageReferences,
  type AuthoredCanvasImagePromptMention,
  type CanvasImagePromptCompilation,
  type CanvasImageReferenceResolution,
} from './canvasImageReferences';

const asset = (
  index: number,
  overrides: Partial<CreativeAsset> = {}
): CreativeAsset => ({
  id: testUuid(index),
  kind: 'image',
  title: `Asset ${index}`,
  collection: null,
  tags: [],
  mimeType: 'image/png',
  width: 1024,
  height: 1024,
  bytes: 1,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/files/${testUuid(index)}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
  ...overrides,
});

const imageNode = (index: number, assetId: string | null) => {
  const node = testNode('image', index);
  node.data.assetId = assetId;
  return node;
};

const panoramaNode = (index: number, assetId: string | null) => {
  const node = testNode('panorama', index);
  node.data.assetId = assetId;
  return node;
};

describe('canvas image reference resolution', () => {
  test('uses connected text as prompt input without consuming image ordinals or limits', () => {
    const target = imageNode(40, null);
    const text = testNode('text', 41);
    text.data.text = '一只猫，水彩风格';
    const image = imageNode(42, testUuid(43));
    const document = testDocument([target, text, image], [testEdge(44, text.id, target.id)]);
    const resolve = () => resolveCanvasImageReferences({ document }, target.id, [asset(43)]);
    const resolution = resolve();
    expect(resolution.issues).toEqual([]);
    expect(resolution.references).toEqual([]);
    expect(resolution.textReferences[0]).toMatchObject({ sourceNodeId: text.id, ordinal: 1, text: text.data.text });
    const compile = (prompt: string, mentions: AuthoredCanvasImagePromptMention[] = []) =>
      compileCanvasImageReferencePrompt(prompt, mentions, resolve().references, resolve().textReferences);
    expect(compile('').providerPrompt).toBe(text.data.text);
    expect(compile('白色背景').providerPrompt).toBe(`${text.data.text}\n\n白色背景`);
    expect(evaluateCanvasImageGenerationGate({ resolution, compilation: compile(''), maxInputImages: 0 })).toMatchObject({
      allowed: true, operation: 't2i', referenceCount: 0,
    });
    document.connections.push(testEdge(45, image.id, target.id));
    const prompt = '@文本1 和 @图片1';
    const mentions = [mentionAt(prompt, '@文本1', text.id), mentionAt(prompt, '@图片1', image.id)];
    expect(compile(prompt, mentions).providerPrompt).toBe(`${text.data.text} 和 Reference 1`);
    text.data.text = '修改后的文字';
    expect(compile(prompt, mentions).providerPrompt).toBe('修改后的文字 和 Reference 1');
    document.connections = document.connections.filter((edge) => edge.sourceNodeId !== text.id);
    expect(compile(prompt, mentions).issues).toMatchObject([{ code: 'mention_reference_disconnected' }]);
  });

  test('reports empty connected text with a text-specific blocker', () => {
    const target = imageNode(46, null);
    const text = testNode('text', 47);
    const resolution = resolveCanvasImageReferences({ document: testDocument([target, text], [testEdge(48, text.id, target.id)]) }, target.id, []);
    expect(resolution.issues).toMatchObject([{ code: 'source_text_empty' }]);
    expect(evaluateCanvasImageGenerationGate({
      resolution, compilation: compileCanvasImageReferencePrompt('画猫', [], [], resolution.textReferences), maxInputImages: 0,
    }).allowed).toBe(false);
  });

  test('derives direct image and panorama inputs in durable connection order', () => {
    const personAsset = asset(101, { title: '人物图' });
    const panoramaAsset = asset(102, { title: '   ' });
    const ignoredAsset = asset(103);
    const target = imageNode(10, null);
    const person = imageNode(11, personAsset.id);
    const panorama = panoramaNode(12, panoramaAsset.id);
    const ignored = imageNode(13, ignoredAsset.id);
    const state = createInitialCanvasState({
      document: testDocument(
        [target, person, panorama, ignored],
        [
          testEdge(201, panorama.id, target.id),
          testEdge(202, target.id, ignored.id),
          testEdge(203, person.id, target.id),
        ]
      ),
    });

    const result = resolveCanvasImageReferences(state, target.id, [
      personAsset,
      panoramaAsset,
      ignoredAsset,
    ]);

    expect(result.inboundConnectionCount).toBe(2);
    expect(result.issues).toEqual([]);
    expect(
      result.references.map((reference) => ({
        ordinal: reference.ordinal,
        providerLabel: reference.providerLabel,
        sourceNodeId: reference.sourceNodeId,
        sourceNodeKind: reference.sourceNodeKind,
        assetId: reference.assetId,
        displayName: reference.displayName,
      }))
    ).toEqual([
      {
        ordinal: 1,
        providerLabel: 'Reference 1',
        sourceNodeId: panorama.id,
        sourceNodeKind: 'panorama',
        assetId: panoramaAsset.id,
        displayName: 'Reference 1',
      },
      {
        ordinal: 2,
        providerLabel: 'Reference 2',
        sourceNodeId: person.id,
        sourceNodeKind: 'image',
        assetId: personAsset.id,
        displayName: '人物图',
      },
    ]);
  });

  test('reports every invalid inbound edge and rejects later duplicate assets', () => {
    const target = imageNode(20, null);
    const unsupported = testNode('video', 21);
    const missingAssetId = imageNode(22, null);
    const unresolved = imageNode(23, testUuid(303));
    const wrongKindAsset = asset(304, { kind: 'video', mimeType: 'video/mp4' });
    const wrongKind = imageNode(24, wrongKindAsset.id);
    const sharedAsset = asset(305, { title: 'Shared' });
    const first = imageNode(25, sharedAsset.id);
    const duplicate = panoramaNode(26, sharedAsset.id);
    const missingSourceNodeId = testUuid(27);
    const connections = [
      testEdge(401, missingSourceNodeId, target.id),
      testEdge(402, unsupported.id, target.id),
      testEdge(403, missingAssetId.id, target.id),
      testEdge(404, unresolved.id, target.id),
      testEdge(405, wrongKind.id, target.id),
      testEdge(406, first.id, target.id),
      testEdge(407, duplicate.id, target.id),
    ];
    const state = createInitialCanvasState({
      document: testDocument(
        [target, unsupported, missingAssetId, unresolved, wrongKind, first, duplicate],
        connections
      ),
    });

    const result = resolveCanvasImageReferences(state, target.id, [
      wrongKindAsset,
      sharedAsset,
    ]);

    expect(result.inboundConnectionCount).toBe(7);
    expect(result.references).toHaveLength(1);
    expect(result.references[0]).toMatchObject({
      ordinal: 1,
      connection: { id: connections[5].id },
      sourceNodeId: first.id,
      assetId: sharedAsset.id,
    });
    expect(result.issues.map((issue) => issue.code)).toEqual([
      'source_node_missing',
      'source_node_kind_unsupported',
      'source_asset_id_missing',
      'source_asset_unresolved',
      'source_asset_kind_unsupported',
      'duplicate_asset',
    ]);
    expect(result.issues.at(-1)).toEqual({
      code: 'duplicate_asset',
      connectionId: connections[6].id,
      sourceNodeId: duplicate.id,
      assetId: sharedAsset.id,
      firstConnectionId: connections[5].id,
      firstSourceNodeId: first.id,
    });
  });

  test('pins the active node image before directly connected references', () => {
    const baseAsset = asset(311, { title: '当前图' });
    const clothingAsset = asset(312, { title: '服装图' });
    const target = imageNode(31, baseAsset.id);
    const clothing = imageNode(32, clothingAsset.id);
    const state = createInitialCanvasState({
      document: testDocument(
        [target, clothing],
        [testEdge(411, clothing.id, target.id)]
      ),
    });

    const result = resolveCanvasImageReferences(state, target.id, [
      clothingAsset,
      baseAsset,
    ]);

    expect(result.issues).toEqual([]);
    expect(
      result.references.map((reference) => ({
        ordinal: reference.ordinal,
        sourceNodeId: reference.sourceNodeId,
        assetId: reference.assetId,
        connectionId: reference.connection?.id ?? null,
      }))
    ).toEqual([
      {
        ordinal: 1,
        sourceNodeId: target.id,
        assetId: baseAsset.id,
        connectionId: null,
      },
      {
        ordinal: 2,
        sourceNodeId: clothing.id,
        assetId: clothingAsset.id,
        connectionId: testUuid(411),
      },
    ]);
  });

  test('ignores only proven config lineage and rejects unrelated config input', () => {
    const resultAsset = asset(330);
    const resultNode = imageNode(331, resultAsset.id);
    const config = testNode('config', 332);
    config.data.resultAssetIds = [resultAsset.id];
    const lineageState = createInitialCanvasState({
      document: testDocument(
        [config, resultNode],
        [testEdge(333, config.id, resultNode.id)]
      ),
    });
    expect(
      resolveCanvasImageReferences(lineageState, resultNode.id, [resultAsset]).issues
    ).toEqual([]);

    const emptyTarget = imageNode(334, null);
    const manualState = createInitialCanvasState({
      document: testDocument(
        [config, emptyTarget],
        [testEdge(335, config.id, emptyTarget.id)]
      ),
    });
    expect(
      resolveCanvasImageReferences(manualState, emptyTarget.id, []).issues.map(
        (issue) => issue.code
      )
    ).toEqual(['source_node_kind_unsupported']);
  });

  test('rejects missing and non-image target nodes before inspecting edges', () => {
    const missingTargetId = testUuid(501);
    const textTarget = testNode('text', 502);
    const state = createInitialCanvasState({ document: testDocument([textTarget]) });

    expect(resolveCanvasImageReferences(state, missingTargetId, []).issues).toEqual([
      { code: 'target_node_missing', targetNodeId: missingTargetId },
    ]);
    expect(resolveCanvasImageReferences(state, textTarget.id, []).issues).toEqual([
      {
        code: 'target_node_kind_unsupported',
        targetNodeId: textTarget.id,
        targetNodeKind: 'text',
      },
    ]);
  });
});

const resolvedFixture = (count = 2): CanvasImageReferenceResolution => {
  const target = imageNode(600, null);
  const sources = Array.from({ length: count }, (_, index) =>
    imageNode(601 + index, testUuid(701 + index))
  );
  const assets = Array.from({ length: count }, (_, index) =>
    asset(701 + index, { title: index === 0 ? '人物' : '服装' })
  );
  const state = createInitialCanvasState({
    document: testDocument(
      [target, ...sources],
      sources.map((source, index) => testEdge(801 + index, source.id, target.id))
    ),
  });
  return resolveCanvasImageReferences(state, target.id, assets);
};

const mentionAt = (
  prompt: string,
  tokenText: string,
  sourceNodeId: string,
  fromIndex = 0
): AuthoredCanvasImagePromptMention => {
  const start = prompt.indexOf(tokenText, fromIndex);
  return { sourceNodeId, start, end: start + tokenText.length, tokenText };
};

describe('canvas image prompt reference compilation', () => {
  test('compiles unsorted and repeated authored mentions without rewriting other text', () => {
    const resolution = resolvedFixture();
    const [person, clothing] = resolution.references;
    const prompt = '让 @人物 穿上 @服装，保持 @人物 的脸部和发型。';
    const firstPerson = mentionAt(prompt, '@人物', person.sourceNodeId);
    const clothingMention = mentionAt(prompt, '@服装', clothing.sourceNodeId);
    const secondPerson = mentionAt(
      prompt,
      '@人物',
      person.sourceNodeId,
      firstPerson.end
    );

    const result = compileCanvasImageReferencePrompt(
      prompt,
      [secondPerson, clothingMention, firstPerson],
      resolution.references
    );

    expect(result).toEqual({
      ok: true,
      authoredPrompt: prompt,
      providerPrompt:
        '让 Reference 1 穿上 Reference 2，保持 Reference 1 的脸部和发型。',
      referencedSourceNodeIds: [person.sourceNodeId, clothing.sourceNodeId],
      issues: [],
    });
  });

  test('fails closed for invalid, stale, overlapping and disconnected mentions', () => {
    const resolution = resolvedFixture(1);
    const [person] = resolution.references;
    const prompt = '@人物 保持脸部';
    const disconnectedNodeId = testUuid(999);
    const result = compileCanvasImageReferencePrompt(
      prompt,
      [
        { sourceNodeId: person.sourceNodeId, start: -1, end: 2, tokenText: '@人' },
        { sourceNodeId: person.sourceNodeId, start: 0, end: 3, tokenText: '@服装' },
        { sourceNodeId: person.sourceNodeId, start: 1, end: 4, tokenText: '人物 ' },
        {
          sourceNodeId: disconnectedNodeId,
          start: 4,
          end: 6,
          tokenText: '保持',
        },
      ],
      resolution.references
    );

    expect(result.ok).toBe(false);
    expect(result.providerPrompt).toBeNull();
    expect(result.issues.map((issue) => issue.code)).toEqual([
      'mention_range_invalid',
      'mention_token_mismatch',
      'mention_ranges_overlap',
      'mention_reference_disconnected',
    ]);
  });
});

const successfulCompilation = (
  resolution: CanvasImageReferenceResolution
): CanvasImagePromptCompilation =>
  compileCanvasImageReferencePrompt('整体描述', [], resolution.references);

describe('canvas image generation gate', () => {
  test('selects t2i without references and permits one unknown-capability input', () => {
    const noReferences = resolvedFixture(0);
    const t2i = evaluateCanvasImageGenerationGate({
      resolution: noReferences,
      compilation: successfulCompilation(noReferences),
      maxInputImages: null,
    });
    expect(t2i).toEqual({
      allowed: true,
      operation: 't2i',
      referenceCount: 0,
      blockers: [],
    });

    const oneReference = resolvedFixture(1);
    expect(
      evaluateCanvasImageGenerationGate({
        resolution: oneReference,
        compilation: successfulCompilation(oneReference),
        maxInputImages: null,
      })
    ).toMatchObject({ allowed: true, operation: 'i2i', referenceCount: 1 });
  });

  test('blocks unknown or exceeded multi-image model limits', () => {
    const resolution = resolvedFixture(2);
    const compilation = successfulCompilation(resolution);

    expect(
      evaluateCanvasImageGenerationGate({
        resolution,
        compilation,
        maxInputImages: null,
      }).blockers
    ).toEqual([{ code: 'reference_limit_unknown', referenceCount: 2 }]);
    expect(
      evaluateCanvasImageGenerationGate({
        resolution,
        compilation,
        maxInputImages: 1,
      }).blockers
    ).toEqual([
      { code: 'reference_limit_exceeded', referenceCount: 2, maxInputImages: 1 },
    ]);
    expect(
      evaluateCanvasImageGenerationGate({
        resolution,
        compilation,
        maxInputImages: 2,
      }).allowed
    ).toBe(true);
  });

  test('surfaces graph and prompt issues as generation blockers', () => {
    const resolution = resolvedFixture(1);
    const graphIssue = {
      code: 'source_asset_unresolved' as const,
      connectionId: testUuid(1_001),
      sourceNodeId: testUuid(1_002),
      assetId: testUuid(1_003),
    };
    const invalidResolution: CanvasImageReferenceResolution = {
      ...resolution,
      issues: [graphIssue],
    };
    const compilation = compileCanvasImageReferencePrompt(
      '@断开',
      [
        {
          sourceNodeId: testUuid(1_004),
          start: 0,
          end: 3,
          tokenText: '@断开',
        },
      ],
      resolution.references
    );

    const gate = evaluateCanvasImageGenerationGate({
      resolution: invalidResolution,
      compilation,
      maxInputImages: 3,
    });

    expect(gate.allowed).toBe(false);
    expect(gate.blockers.map((blocker) => blocker.code)).toEqual([
      'reference_resolution_failed',
      'prompt_compilation_failed',
    ]);
  });

  test('blocks a known aggregate reference payload above the product byte budget', () => {
    const resolution = resolvedFixture(2);
    resolution.references[0].asset.bytes = MAX_CANVAS_IMAGE_REFERENCE_BYTES;
    resolution.references[1].asset.bytes = 1;
    const gate = evaluateCanvasImageGenerationGate({
      resolution,
      compilation: successfulCompilation(resolution),
      maxInputImages: 8,
    });
    expect(gate.blockers).toEqual([
      {
        code: 'reference_bytes_exceeded',
        totalBytes: MAX_CANVAS_IMAGE_REFERENCE_BYTES + 1,
        maxInputBytes: MAX_CANVAS_IMAGE_REFERENCE_BYTES,
      },
    ]);
  });
});
