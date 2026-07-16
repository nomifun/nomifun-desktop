/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  getUpdateAvailabilitySnapshot,
  reportNoUpdateAvailable,
  reportUpdateAvailable,
} from './useUpdateAvailability';

describe('shared update availability', () => {
  test('publishes available versions and clears them after a no-update result', () => {
    reportNoUpdateAvailable();
    expect(getUpdateAvailabilitySnapshot()).toEqual({ available: false });

    reportUpdateAvailable('0.2.22');
    expect(getUpdateAvailabilitySnapshot()).toEqual({ available: true, version: '0.2.22' });

    reportNoUpdateAvailable();
    expect(getUpdateAvailabilitySnapshot()).toEqual({ available: false });
  });

  test('supports update events that do not include a version', () => {
    reportUpdateAvailable();
    expect(getUpdateAvailabilitySnapshot()).toEqual({ available: true });

    reportNoUpdateAvailable();
  });
});
