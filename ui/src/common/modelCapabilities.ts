/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ModelTrait } from '@/common/protocolBindings/ModelTrait';

/** Canonical product order for every endpoint-determining model capability. */
export const MODEL_TASK_ORDER = [
  'chat',
  'realtime_conversation',
  'image_generation',
  'image_edit',
  'video_generation',
  'speech_synthesis',
  'speech_recognition',
  'embedding',
  'rerank',
] as const satisfies readonly ModelTask[];

/** Canonical product order for refinements within a model capability. */
export const MODEL_TRAIT_ORDER = [
  'vision_input',
  'video_input',
  'audio_input',
  'audio_output',
  'realtime',
  'streaming',
  'function_calling',
  'reasoning',
  'web_search',
] as const satisfies readonly ModelTrait[];
