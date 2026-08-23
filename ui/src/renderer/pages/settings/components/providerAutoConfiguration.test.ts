/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { ProtocolDescriptor } from '@/common/types/provider/modelProtocolManifest';
import {
  applyProviderAutoConfiguration,
  applyProviderCompatibilityMode,
  buildProviderAutoConfigurationTargets,
  DEFAULT_REQUIRED_OUTPUT_LIMIT,
  isAutoConfigurationPlatform,
  normalizeProviderBaseUrlForCompatibilityMode,
  providerCompatibilityAuthScheme,
  providerCompatibilityProtocolForTask,
  providerCompatibilityProtocolPreferences,
  selectProbeAuthScheme,
  selectProviderAutoConfiguration,
} from './providerAutoConfiguration';
import {
  emptyCapabilityDraft,
  type ModelProtocolManifest,
} from './providerModelAdvanced';

const descriptor = (
  protocolId: string,
  authSchemes: string[],
  requiresOutputCeiling = false
): ProtocolDescriptor => ({
  protocol_id: protocolId,
  supported_tasks: ['chat'],
  executor: 'agent',
  transport: 'http',
  requires_output_ceiling: requiresOutputCeiling,
  allowed_auth_schemes: authSchemes,
  scopes: ['custom'],
  platforms: [],
  default_connections: [],
  endpoints: [
    {
      task: 'chat',
      field: 'endpoint',
      purpose: 'submit',
      method: 'POST',
      default_value: '/chat',
      root_shape: 'versioned_root',
      allowed_placeholders: [],
      required_placeholders: [],
      editable: true,
    },
  ],
  root_shape: 'versioned_root',
});

const manifest = (
  protocols: ProtocolDescriptor[],
  recommendation = protocols[0]?.protocol_id
): ModelProtocolManifest => ({
  tasks: ['chat'],
  preset: 'custom',
  platform: 'custom',
  requested_task: 'chat',
  platform_default_base_url: null,
  requires_user_input: true,
  default_auth_scheme: 'bearer',
  auth_schemes: [],
  recommendation: recommendation
    ? {
        protocol_id: recommendation,
        connection_role: null,
        default_base_url: null,
        default_auth_scheme: 'bearer',
        base_url_override_required: false,
      }
    : null,
  protocols,
});

describe('provider auto configuration', () => {
  test('is limited to Custom and New API', () => {
    expect(isAutoConfigurationPlatform('custom')).toBe(true);
    expect(isAutoConfigurationPlatform('new-api')).toBe(true);
    expect(isAutoConfigurationPlatform('OpenAI')).toBe(false);
  });

  test('keeps a compatible user scheme and concretizes parameterized schemes', () => {
    const generic = descriptor('generic.chat', [
      'bearer',
      'header_key:<name>',
      'query_key:<param>',
    ]);
    expect(selectProbeAuthScheme(generic, 'header_key:x-company-key')).toBe(
      'header_key:x-company-key'
    );
    expect(selectProbeAuthScheme(generic, '')).toBe('bearer');
    expect(
      selectProbeAuthScheme(descriptor('header-only', ['header_key:<name>']), '')
    ).toBe('header_key:x-api-key');
  });

  test('provides complete OpenAI and Claude compatibility presets', () => {
    expect(providerCompatibilityAuthScheme('auto')).toBeUndefined();
    expect(providerCompatibilityAuthScheme('openai')).toBe('bearer');
    expect(providerCompatibilityAuthScheme('anthropic')).toBe('header_key:x-api-key');
    expect(providerCompatibilityProtocolForTask('openai', 'chat')).toBe(
      'openai.chat_text'
    );
    expect(providerCompatibilityProtocolForTask('openai', 'image_generation')).toBe(
      'openai.images'
    );
    expect(providerCompatibilityProtocolForTask('anthropic', 'chat')).toBe(
      'anthropic.messages'
    );
    expect(
      providerCompatibilityProtocolForTask('anthropic', 'image_generation')
    ).toBeUndefined();
  });

  test('restricts live probing to the explicitly selected interface mode', () => {
    const openai = descriptor('openai.chat_text', ['bearer']);
    const anthropic = descriptor('anthropic.messages', ['header_key:x-api-key'], true);
    const definition = applyProviderCompatibilityMode(
      {
        model: 'claude-compatible-model',
        capabilities: [emptyCapabilityDraft('chat')],
      },
      'anthropic',
      true
    );
    const targets = buildProviderAutoConfigurationTargets(
      definition,
      { chat: manifest([openai, anthropic]) },
      'header_key:x-api-key',
      false,
      providerCompatibilityProtocolPreferences('anthropic')
    );
    expect(targets).toHaveLength(1);
    expect(targets[0]?.candidates.map((candidate) => candidate.descriptor.protocol_id)).toEqual([
      'anthropic.messages',
    ]);
    expect(targets[0]?.candidates[0]?.authScheme).toBe('header_key:x-api-key');
  });

  test('switches presets in one step and resets adapter-owned fields', () => {
    const customized = {
      model: 'm',
      capabilities: [
        {
          ...emptyCapabilityDraft('chat'),
          transportSource: 'user' as const,
          protocol: 'openai.chat_text',
          endpoint: '/custom/chat',
          providerParamsJson: '{"temperature":0.2}',
        },
      ],
    };
    const claude = applyProviderCompatibilityMode(customized, 'anthropic', true);
    expect(claude.capabilities[0]).toMatchObject({
      transportSource: 'user',
      protocol: 'anthropic.messages',
      endpoint: '',
      providerParamsJson: '',
      outputLimit: DEFAULT_REQUIRED_OUTPUT_LIMIT,
    });

    const openai = applyProviderCompatibilityMode(claude, 'openai', true);
    expect(openai.capabilities[0]).toMatchObject({
      protocol: 'openai.chat_text',
      outputLimit: undefined,
    });
  });

  test('returning to auto clears an explicit preset so detection can own it again', () => {
    const claude = applyProviderCompatibilityMode(
      {
        model: 'm',
        capabilities: [emptyCapabilityDraft('chat')],
      },
      'anthropic',
      true
    );
    const automatic = applyProviderCompatibilityMode(claude, 'auto', true);
    expect(automatic.capabilities[0]).toMatchObject({
      transportSource: 'blank',
      protocol: '',
      outputLimit: undefined,
    });
  });

  test('Claude mode does not silently install OpenAI transport on unsupported tasks', () => {
    const imageRecommendation = {
      ...emptyCapabilityDraft('image_generation'),
      transportSource: 'recommendation' as const,
      protocol: 'openai.images',
    };
    const result = applyProviderCompatibilityMode(
      { model: 'm', capabilities: [imageRecommendation] },
      'anthropic'
    );
    expect(result.capabilities[0]).toMatchObject({
      transportSource: 'blank',
      protocol: '',
    });
  });

  test('normalizes provider roots for OpenAI and Claude wire formats', () => {
    expect(
      normalizeProviderBaseUrlForCompatibilityMode(
        'https://gateway.example',
        'openai'
      )
    ).toBe('https://gateway.example/v1');
    expect(
      normalizeProviderBaseUrlForCompatibilityMode(
        'https://gateway.example/api/v1',
        'anthropic'
      )
    ).toBe('https://gateway.example/api');
    expect(
      normalizeProviderBaseUrlForCompatibilityMode(
        'https://gateway.example/api',
        'anthropic'
      )
    ).toBe('https://gateway.example/api');
  });

  test('prefers a reachable protocol and adopts its working root and auth', () => {
    const openai = descriptor('openai.chat_text', ['bearer']);
    const anthropic = descriptor('anthropic.messages', ['header_key:x-api-key'], true);
    const target = buildProviderAutoConfigurationTargets(
      {
        model: 'third-party-model',
        capabilities: [emptyCapabilityDraft('chat')],
      },
      { chat: manifest([openai, anthropic]) },
      'bearer',
      false
    )[0]!;
    const selected = selectProviderAutoConfiguration(target, [
      {
        candidate: target.candidates[0]!,
        response: {
          reachability: 'unreachable',
          protocol: 'openai.chat_text',
          task: 'chat',
          root_shape: 'versioned_root',
          attempted_url: 'https://gateway.example/v1/chat/completions',
          elapsed_ms: 10,
          candidates: [],
        },
      },
      {
        candidate: target.candidates[1]!,
        response: {
          reachability: 'unreachable',
          protocol: 'anthropic.messages',
          task: 'chat',
          root_shape: 'origin_root',
          attempted_url: 'https://gateway.example/v1/v1/messages',
          elapsed_ms: 12,
          suggested_base_url: 'https://gateway.example',
          candidates: [
            {
              base_url: 'https://gateway.example',
              attempted_url: 'https://gateway.example/v1/messages',
              reachability: 'reachable',
              http_status: 400,
            },
          ],
        },
      },
    ]);

    expect(selected).toEqual({
      task: 'chat',
      protocol: 'anthropic.messages',
      authScheme: 'header_key:x-api-key',
      confidence: 'verified',
      suggestedBaseUrl: 'https://gateway.example',
      outputLimit: DEFAULT_REQUIRED_OUTPUT_LIMIT,
    });
  });

  test('installs the preferred fallback when the endpoint cannot be confirmed', () => {
    const openai = descriptor('openai.chat_text', ['bearer']);
    const target = buildProviderAutoConfigurationTargets(
      {
        model: '',
        capabilities: [emptyCapabilityDraft('chat')],
      },
      { chat: manifest([openai]) },
      'bearer',
      false
    )[0]!;
    expect(selectProviderAutoConfiguration(target, [{ candidate: target.candidates[0]! }])).toEqual({
      task: 'chat',
      protocol: 'openai.chat_text',
      authScheme: 'bearer',
      confidence: 'fallback',
    });
  });

  test('fills untouched transport but preserves user and persisted configuration', () => {
    const blank = emptyCapabilityDraft('chat');
    const detected = {
      task: 'chat' as const,
      protocol: 'anthropic.messages',
      authScheme: 'header_key:x-api-key',
      confidence: 'verified' as const,
      outputLimit: DEFAULT_REQUIRED_OUTPUT_LIMIT,
    };
    const applied = applyProviderAutoConfiguration(
      { model: 'm', capabilities: [blank] },
      [detected]
    );
    expect(applied.capabilities[0]).toMatchObject({
      transportSource: 'recommendation',
      protocol: 'anthropic.messages',
      connectionRole: 'default',
      outputLimit: DEFAULT_REQUIRED_OUTPUT_LIMIT,
    });

    for (const transportSource of ['user', 'persisted'] as const) {
      const protectedDefinition = {
        model: 'm',
        capabilities: [
          {
            ...blank,
            transportSource,
            protocol: 'openai.chat_text',
            endpoint: '/custom',
          },
        ],
      };
      expect(
        applyProviderAutoConfiguration(protectedDefinition, [detected])
      ).toBe(protectedDefinition);
    }
  });
});
