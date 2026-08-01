/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { createBasicRuntimeChat } from '@renderer/pages/conversation/platforms/BasicRuntimeChat';
import RemoteSendBox from './RemoteSendBox';

/** Remote-agent chat surface — a parameterization of the shared BasicRuntimeChat. */
export default createBasicRuntimeChat('remote', RemoteSendBox);
