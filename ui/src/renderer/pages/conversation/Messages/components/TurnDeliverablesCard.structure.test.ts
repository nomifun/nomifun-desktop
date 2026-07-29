/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const cardSource = readFileSync(new URL('./TurnDeliverablesCard.tsx', import.meta.url), 'utf8');
const listSource = readFileSync(new URL('../MessageList.tsx', import.meta.url), 'utf8');

describe('TurnDeliverablesCard structure', () => {
  test('never renders an empty or unverified card', () => {
    // Availability must settle before anything is shown, and zero trustworthy
    // items must render nothing.
    expect(cardSource.includes('if (pending || available.length === 0) return null;')).toBe(true);
    // Reported items are gated on a backend existence probe; only committed
    // receipts skip the client-side probe.
    expect(cardSource.includes("if (item.tier === 'receipt') return { ...item, statPath };")).toBe(true);
    expect(cardSource.includes('probeAvailability(statPath, workspace)')).toBe(true);
  });

  test('hides dead action buttons on degraded surfaces', () => {
    // No preview affordance without a PreviewProvider (read-only projections).
    expect(cardSource.includes('canPreview && isPreviewSupportedExt(item.fileName)')).toBe(true);
    // Shell open/reveal execute on the backend host; hide them in WebUI mode.
    expect(cardSource.includes('isDesktopShell()')).toBe(true);
  });

  test('truncates to the first three files with an explicit reveal control', () => {
    expect(cardSource.includes('DEFAULT_VISIBLE_COUNT = 3')).toBe(true);
    expect(cardSource.includes('available.slice(0, DEFAULT_VISIBLE_COUNT)')).toBe(true);
    expect(cardSource.includes("aria-expanded={showAll}")).toBe(true);
    expect(cardSource.includes('messages.turnDeliverables.showMore')).toBe(true);
    expect(cardSource.includes('messages.turnDeliverables.showLess')).toBe(true);
  });

  test('gives the filename a wider share of the available row space', () => {
    expect(cardSource.includes("className='flex flex-1 items-center gap-8px min-w-0'")).toBe(true);
    expect(cardSource.includes('min-w-0 max-w-60% truncate text-14px text-t-primary')).toBe(true);
    expect(cardSource.includes('shrink-0 max-w-40%')).toBe(false);
  });

  test('message list mounts the card once per turn behind the deliverables model', () => {
    expect(listSource.includes('collectTurnDeliverables')).toBe(true);
    expect(listSource.includes("type: 'turn_deliverables'")).toBe(true);
    // Stable anchor id keyed by turn identity (React key + jump-target contract).
    expect(listSource.includes('`turn-deliverables-${turnId}`')).toBe(true);
    // The card gates on the same turn lifecycle the disclosure header uses.
    expect(listSource.includes('turnGates.set(entry.msg_id, { running: entry.running, state: entry.state })')).toBe(
      true
    );
  });

  test('places the final assistant actions after the turn deliverables card', () => {
    expect(listSource.includes("type: 'turn_actions'")).toBe(true);
    expect(listSource.includes('`turn-actions-${turnId}`')).toBe(true);
    expect(listSource.indexOf("type: 'turn_deliverables'")).toBeLessThan(
      listSource.lastIndexOf("type: 'turn_actions'")
    );
    expect(listSource.includes('<MessageText message={item.message} actionsOnly />')).toBe(true);
    expect(listSource.includes('movedActionMessageIds.has((item as TMessage).id)')).toBe(true);
  });
});
