/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { plugin } from 'bun';
import { pathToFileURL } from 'node:url';

// Vite exposes imported images as URLs. Give Bun the same module shape so
// component tests can load renderer assets without parsing their image bytes.
plugin({
  name: 'renderer-test-image-assets',
  setup(build) {
    build.onLoad({ filter: /[\\/]renderer[\\/]assets[\\/].*\.(?:png|svg)$/ }, ({ path }) => ({
      loader: 'object',
      exports: { default: pathToFileURL(path).href },
    }));
  },
});
