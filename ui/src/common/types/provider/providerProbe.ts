/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Provider-level connection probe contract.
 *
 * The per-model health check needs a persisted `(provider, model, task)`
 * capability, so a newly created provider could not be validated until
 * something was already built on top of it. These routes answer the prior
 * question — is this address an API, and does this key reach it — and do so as
 * three states, because "the URL is right but the key is not" is the most
 * common real answer and a boolean cannot express it.
 */

export type { EndpointRootShape } from '@/common/protocolBindings/EndpointRootShape';
export type { ProbeCandidateResult } from '@/common/protocolBindings/ProbeCandidateResult';
export type { ProbeProviderConnectionAnonymousRequest } from '@/common/protocolBindings/ProbeProviderConnectionAnonymousRequest';
export type { ProbeProviderConnectionRequest } from '@/common/protocolBindings/ProbeProviderConnectionRequest';
export type { ProbeProviderConnectionResponse } from '@/common/protocolBindings/ProbeProviderConnectionResponse';
export type { ProviderReachability } from '@/common/protocolBindings/ProviderReachability';
