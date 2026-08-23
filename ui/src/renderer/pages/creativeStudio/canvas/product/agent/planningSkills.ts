/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export const CREATIVE_STUDIO_PLANNING_SKILLS = [
  {
    id: 'creative-studio-canvas',
    labelKey: 'creativeStudio.agent.skills.canvas.label',
    descriptionKey: 'creativeStudio.agent.skills.canvas.description',
  },
  {
    id: 'creative-studio-organize',
    labelKey: 'creativeStudio.agent.skills.organize.label',
    descriptionKey: 'creativeStudio.agent.skills.organize.description',
  },
  {
    id: 'creative-studio-template',
    labelKey: 'creativeStudio.agent.skills.template.label',
    descriptionKey: 'creativeStudio.agent.skills.template.description',
  },
] as const;

export type CreativeStudioPlanningSkillId =
  (typeof CREATIVE_STUDIO_PLANNING_SKILLS)[number]['id'];

export const DEFAULT_CREATIVE_STUDIO_PLANNING_SKILL_IDS = [
  'creative-studio-canvas',
] as const satisfies readonly CreativeStudioPlanningSkillId[];

export const isCreativeStudioPlanningSkillId = (
  value: string
): value is CreativeStudioPlanningSkillId =>
  CREATIVE_STUDIO_PLANNING_SKILLS.some((skill) => skill.id === value);
