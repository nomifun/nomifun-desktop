/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export {
  CREATIVE_STUDIO_TEMPLATES_ENDPOINT,
  CreativeTemplateContractError,
  createCreativeTemplateApi,
  creativeTemplateApi,
  isCreativeTemplateContractError,
  parseTemplateDefinition,
} from './templateApi';
export type {
  CreativeTemplateApi,
  CreativeTemplateHttpRequest,
  SaveCreativeTemplateRequest,
} from './templateApi';
export {
  CreativeTemplateRepositoryError,
  createCreativeTemplateRepository,
  creativeTemplateRepository,
  isCreativeTemplateRepositoryError,
  toCreativeTemplateRepositoryError,
} from './templateRepository';
export type {
  CreativeTemplateRepository,
  CreativeTemplateRepositoryErrorKind,
} from './templateRepository';
export {
  CREATIVE_STUDIO_TEMPLATE_RUNS_ENDPOINT,
  createCreativeTemplateRunApi,
  creativeTemplateRunApi,
} from './templateRunApi';
export type {
  CreateCreativeTemplateRunRequest,
  CreativeTemplateRunApi,
  SaveCreativeTemplateRunRequest,
} from './templateRunApi';
