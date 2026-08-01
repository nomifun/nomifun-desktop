/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { createBasicRuntimeChat } from '@renderer/pages/conversation/platforms/BasicRuntimeChat';
import OpenClawSendBox from './OpenClawSendBox';

/** OpenClaw gateway chat surface — a parameterization of the shared BasicRuntimeChat. */
export default createBasicRuntimeChat('openclaw-gateway', OpenClawSendBox);
