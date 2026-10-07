/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { GlobalRegistrator } from '@happy-dom/global-registrator';
import { initializeAgentBrowserStorageGeneration } from '../src/common/utils/browserStorageKey';

type ReactActGlobal = typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean;
};

if (!GlobalRegistrator.isRegistered) {
  GlobalRegistrator.register({ url: 'http://127.0.0.1/' });
}

(globalThis as ReactActGlobal).IS_REACT_ACT_ENVIRONMENT = true;
// Standalone UI tests have an explicit backend-generation fixture, rather
// than a production fallback or another copy of the canonical Rust version.
initializeAgentBrowserStorageGeneration(1);

// The test process owns this DOM for its full lifetime. React and Arco can
// schedule work after a test file's hooks finish, so per-file unregistering
// would remove `window` while those callbacks are still draining.
