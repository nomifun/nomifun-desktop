/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { buildAuthSchemeEditPatch } from './providerEditForm';

describe('provider auth-scheme edit patch', () => {
  test('does not overwrite persisted auth when the detail form was not edited', () => {
    expect(buildAuthSchemeEditPatch('header_key:x-api-key', 'bearer', false)).toEqual({});
  });

  test('persists an explicit user change and normalizes surrounding whitespace', () => {
    expect(buildAuthSchemeEditPatch('bearer', ' token ', true)).toEqual({
      auth_scheme: 'token',
    });
  });

  test('omits a dirty but unchanged value', () => {
    expect(buildAuthSchemeEditPatch('header_key:x-api-key', ' header_key:x-api-key ', true)).toEqual({});
  });
});
