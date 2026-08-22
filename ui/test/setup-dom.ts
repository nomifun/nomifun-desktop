/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { GlobalRegistrator } from '@happy-dom/global-registrator';

type ReactActGlobal = typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean;
};

if (!GlobalRegistrator.isRegistered) {
  GlobalRegistrator.register({ url: 'http://127.0.0.1/' });
}

(globalThis as ReactActGlobal).IS_REACT_ACT_ENVIRONMENT = true;

// The test process owns this DOM for its full lifetime. React and Arco can
// schedule work after a test file's hooks finish, so per-file unregistering
// would remove `window` while those callbacks are still draining.
