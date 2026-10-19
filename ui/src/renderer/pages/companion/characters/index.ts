/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import Mochi from './Mochi';
import Bolt from './Bolt';
import { customDeskSpec } from './customDesk';
import type { CharacterDeskSpec, CharacterMeta, CustomFigureMeta } from './types';

export type {
  CharacterDeskSpec,
  CharacterMeta,

  CustomFigureMeta,
  CompanionActivity,
  CompanionMood,
} from './types';

export const DEFAULT_CHARACTER_ID = 'mochi';

const DEFAULT_CHARACTER: CharacterMeta = {
  id: DEFAULT_CHARACTER_ID,
  palette: ['#fff6f0', '#ffb7c9'],
  Component: Mochi,
};

/**
 * The character roster. Order = display order in the picker.
 * `palette` feeds the little swatch chip on each picker card.
 */
export const CHARACTERS: readonly CharacterMeta[] = [
  DEFAULT_CHARACTER,
  { id: 'bolt', palette: ['#bfeee0', '#37e0ff'], Component: Bolt },
];

export const getCharacter = (id?: string | null): CharacterMeta =>
  CHARACTERS.find((c) => c.id === id) ?? DEFAULT_CHARACTER;

/**
 * The compact desktop window shared by the built-in characters.
 * Must match the creation-time `inner_size` in apps/desktop/src/main.rs —
 * applyDeskSize self-heals a mismatch with a visible startup resize.
 * Height = figure (120) + chrome (60: hover chat bar reserve + hop headroom);
 * the bubble's room is grown on demand (enterChatSize), not reserved here, so the
 * idle window hugs the figure instead of parking a tall transparent strip on the
 * desktop.
 */
export const DEFAULT_DESK: CharacterDeskSpec = { windowWidth: 200, windowHeight: 180, figureHeight: 120 };

export const CUSTOM_CHARACTER_ID = 'custom';

/** Custom figures use their configured dimensions; all other looks use the compact window. */
export const getDeskSpecFor = (id?: string | null, meta?: CustomFigureMeta | null): CharacterDeskSpec =>
  id === CUSTOM_CHARACTER_ID && meta ? customDeskSpec(meta) : DEFAULT_DESK;
