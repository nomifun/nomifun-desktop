/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { createCanvasId } from './document';
import { connectCanvasNodes, validateCanvasConnection } from './graph';
import { testDocument, testEdge, testNode, testUuid } from './testFixtures';

describe('Creative Studio graph constraints', () => {
  const image = testNode('image', 1);
  const panorama = testNode('panorama', 2);
  const text = testNode('text', 3);
  const firstConfig = testNode('config', 4);
  const secondConfig = testNode('config', 5);
  const director = testNode('director', 6);
  const group = testNode('group', 7);
  const document = testDocument([
    image,
    panorama,
    text,
    firstConfig,
    secondConfig,
    director,
    group,
  ]);

  test('mints canonical bare UUIDv7 ids from the shared generator', () => {
    const id = createCanvasId('node');
    expect(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id)).toBe(true);
    expect(id.includes('creative-studio')).toBe(false);
  });

  test('rejects missing endpoints and self-connections', () => {
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: testUuid(999),
        targetNodeId: text.id,
      })
    ).toEqual({ ok: false, code: 'missing_source' });
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: text.id,
        targetNodeId: testUuid(999),
      })
    ).toEqual({ ok: false, code: 'missing_target' });
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: text.id,
        targetNodeId: text.id,
      })
    ).toEqual({ ok: false, code: 'self_connection' });
  });

  test('rejects duplicate directed edges', () => {
    const withConnection = testDocument(
      document.nodes,
      [testEdge(20, text.id, image.id)]
    );
    expect(
      validateCanvasConnection(withConnection, {
        sourceNodeId: text.id,
        targetNodeId: image.id,
      })
    ).toEqual({ ok: false, code: 'duplicate_connection' });
  });

  test('keeps groups out of the generation graph', () => {
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: group.id,
        targetNodeId: image.id,
      })
    ).toEqual({ ok: false, code: 'group_connection' });
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: image.id,
        targetNodeId: group.id,
      })
    ).toEqual({ ok: false, code: 'group_connection' });
  });

  test('rejects config-to-config edges', () => {
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: firstConfig.id,
        targetNodeId: secondConfig.id,
      })
    ).toEqual({ ok: false, code: 'config_to_config' });
  });

  test('treats Director as image-input-only', () => {
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: director.id,
        targetNodeId: image.id,
      })
    ).toEqual({ ok: false, code: 'director_output_not_supported' });
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: text.id,
        targetNodeId: director.id,
      })
    ).toEqual({ ok: false, code: 'director_requires_image_input' });
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: image.id,
        targetNodeId: director.id,
      })
    ).toEqual({ ok: true });
    expect(
      validateCanvasConnection(document, {
        sourceNodeId: panorama.id,
        targetNodeId: director.id,
      })
    ).toEqual({ ok: true });
  });

  test('creates canonical nullable handles after validation', () => {
    expect(
      connectCanvasNodes(
        document,
        { sourceNodeId: text.id, targetNodeId: image.id },
        { edgeId: testUuid(30) }
      )
    ).toEqual({
      ok: true,
      edge: {
        id: testUuid(30),
        sourceNodeId: text.id,
        targetNodeId: image.id,
        sourceHandle: null,
        targetHandle: null,
      },
    });
  });
});
