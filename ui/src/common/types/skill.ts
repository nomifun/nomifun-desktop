/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

type SkillSource = 'builtin' | 'custom';

export type SkillInfo = {
  name: string;
  description: string;
  name_i18n?: Record<string, string>;
  description_i18n?: Record<string, string>;
  location: string;
  relative_location?: string;
  is_custom: boolean;
  source: SkillSource;
  audience_tags?: string[];
  scenario_tags?: string[];
  /** Library content may remain manageable even when it cannot enter a Session. */
  session_available?: boolean;
  session_error?: string | null;
};
