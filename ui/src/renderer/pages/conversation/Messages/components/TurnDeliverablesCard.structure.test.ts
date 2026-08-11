/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const cardSource = readFileSync(new URL('./TurnDeliverablesCard.tsx', import.meta.url), 'utf8');
const imageCardSource = readFileSync(new URL('./VerifiedImageArtifactCard.tsx', import.meta.url), 'utf8');
const modelSource = readFileSync(new URL('../turnDeliverablesModel.ts', import.meta.url), 'utf8');
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

  test('truncates files and full-resolution images to three with explicit reveal controls', () => {
    expect(cardSource.includes('DEFAULT_VISIBLE_COUNT = 3')).toBe(true);
    expect(cardSource.includes('fileDeliverables.slice(0, DEFAULT_VISIBLE_COUNT)')).toBe(true);
    expect(cardSource.includes('verifiedImages.slice(0, DEFAULT_VISIBLE_COUNT)')).toBe(true);
    expect(cardSource.includes("aria-expanded={showAll}")).toBe(true);
    expect(cardSource.includes("aria-expanded={showAllImages}")).toBe(true);
    expect(cardSource.includes('messages.turnDeliverables.showMore')).toBe(true);
    expect(cardSource.includes('messages.turnDeliverables.showLess')).toBe(true);
    expect(cardSource.includes('messages.turnDeliverables.showMoreImages')).toBe(true);
    expect(cardSource.includes('messages.turnDeliverables.showLessImages')).toBe(true);
  });

  test('renders verified image receipts as first-class cards outside the file rows', () => {
    expect(modelSource.includes("item.tier === 'receipt'")).toBe(true);
    expect(modelSource.includes("item.artifactKind === 'image'")).toBe(true);
    expect(modelSource.includes("item.mimeType.toLowerCase().startsWith('image/')")).toBe(true);
    expect(cardSource.includes('available.filter(isVerifiedImageDeliverable)')).toBe(true);
    expect(cardSource.includes('!isVerifiedImageDeliverable(item)')).toBe(true);
    expect(cardSource.includes('<VerifiedImageArtifactCard')).toBe(true);
    expect(cardSource.includes("key={item.artifactId}")).toBe(true);
    expect(imageCardSource.includes("data-testid='verified-image-artifact-card'")).toBe(true);
    expect(imageCardSource.includes('<LocalImageView')).toBe(true);
  });

  test('image actions use only the persisted artifact path', () => {
    expect(imageCardSource.includes('item.absolutePath ?? item.statPath ?? item.relativePath')).toBe(true);
    expect(imageCardSource.includes('downloadFileFromPath(item.statPath, item.fileName, workspace)')).toBe(true);
    expect(imageCardSource.includes('await copyText(copyTarget)')).toBe(true);
    expect(imageCardSource.includes('launchPreview({')).toBe(true);
    expect(imageCardSource.includes("from '@/renderer/components/Markdown'")).toBe(false);
    expect(imageCardSource.includes('http://')).toBe(false);
    expect(imageCardSource.includes('https://')).toBe(false);
  });

  test('uses compact padding across the card header, file rows and reveal control', () => {
    expect(cardSource.includes('gap-8px px-12px py-8px select-none')).toBe(true);
    expect(cardSource.includes('gap-8px px-12px py-6px hover:bg-3 transition-colors')).toBe(true);
    expect(cardSource.includes('gap-8px px-12px py-6px text-13px text-t-secondary')).toBe(true);
  });

  test('renders one relative path with muted directories and a primary filename', () => {
    expect(cardSource.includes("className='flex flex-1 items-center gap-8px min-w-0'")).toBe(true);
    expect(cardSource.includes('splitFileDisplayPath(item.relativePath, item.fileName)')).toBe(true);
    expect(
      cardSource.includes(
        "const DIRECTORY_PATH_COLOR = 'color-mix(in srgb, var(--text-secondary) 82%, var(--bg-base))'"
      )
    ).toBe(true);
    expect(cardSource.includes("className='min-w-0 truncate' style={{ color: DIRECTORY_PATH_COLOR }}")).toBe(true);
    expect(cardSource.includes("'truncate text-t-primary'")).toBe(true);
    expect(cardSource.includes('secondaryPath')).toBe(false);
    expect(cardSource.includes('key={item.absolutePath ?? item.relativePath}')).toBe(true);
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
