/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Generated backend manifest contract plus the HTTP query shape. */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';

export type { AuthSchemeDescriptor } from '@/common/protocolBindings/AuthSchemeDescriptor';
export type { EndpointRootShape } from '@/common/protocolBindings/EndpointRootShape';
export type { ModelProtocolManifestResponse } from '@/common/protocolBindings/ModelProtocolManifestResponse';
export type { PlatformPresetDescriptor } from '@/common/protocolBindings/PlatformPresetDescriptor';
export type { ProtocolDefaultConnection } from '@/common/protocolBindings/ProtocolDefaultConnection';
export type { ProtocolDescriptor } from '@/common/protocolBindings/ProtocolDescriptor';
export type { ProtocolEndpointDescriptor } from '@/common/protocolBindings/ProtocolEndpointDescriptor';
export type { ProtocolEndpointPurpose } from '@/common/protocolBindings/ProtocolEndpointPurpose';
export type { ProtocolExecutorKind } from '@/common/protocolBindings/ProtocolExecutorKind';
export type { ProtocolRecommendation } from '@/common/protocolBindings/ProtocolRecommendation';
export type { ProtocolScope } from '@/common/protocolBindings/ProtocolScope';
export type { ProtocolTaskDescriptor } from '@/common/protocolBindings/ProtocolTaskDescriptor';
export type { ProtocolTransportKind } from '@/common/protocolBindings/ProtocolTransportKind';

export interface ModelProtocolManifestRequest {
  /** Preset value for create, or stored canonical platform for edit. */
  preset: string;
  task: ModelTask;
  /** Existing provider URL hint disambiguates regional presets sharing a platform. */
  base_url?: string;
  /** Selected Custom model id; enables a task-scoped compatibility recommendation. */
  model?: string;
}
