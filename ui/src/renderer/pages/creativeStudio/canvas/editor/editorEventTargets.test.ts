/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { describe, expect, test } from 'bun:test';

import { canvasNodeIdFromEventTarget } from './editorModel';

describe('canvas delegated event targets', () => {
  test('recovers a node from nested HTML and SVG targets', () => {
    const placement = document.createElement('div');
    placement.dataset.canvasNodeId = '  text-node  ';
    placement.dataset.canvasNodeKind = 'text';
    const content = document.createElement('span');
    const icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    content.append(icon);
    placement.append(content);
    document.body.append(placement);

    expect(canvasNodeIdFromEventTarget(content)).toBe('text-node');
    expect(canvasNodeIdFromEventTarget(icon)).toBe('text-node');
    expect(canvasNodeIdFromEventTarget(document.body)).toBeNull();
    expect(canvasNodeIdFromEventTarget(document.createTextNode('text'))).toBeNull();

    placement.remove();
  });
});
