/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import {
  InvalidEntityIdError,
  conversationTarget,
  isSameSessionTarget,
  parseCompanionEvolutionFeedbackId,
  parseConversationId,
  parseFigureId,
  parsePresetTagId,
  parsePublicAgentAuditEntryId,
  parseTerminalId,
  parseWorkshopEdgeId,
  parseWorkshopNodeId,
  terminalTarget,
  tryParseEntityId,
} from './ids';

describe('entity ids', () => {
  test('strict parsers accept only canonical non-empty strings', () => {
    const validConversation = 'conv_0190f5fe-7c00-7a00-8000-000000000001';
    expect(parseConversationId(validConversation)).toBe(validConversation);
    for (const value of [
      1,
      'conv_0190f5fe-7c00-8a00-8000-000000000001',
      'conv_0190f5fe-7c00-7a00-8000-000000000001 ',
      'conv_01',
      '',
    ]) {
      let error: unknown;
      try {
        parseConversationId(value);
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof InvalidEntityIdError).toBe(true);
    }
    expect(tryParseEntityId('conversation', null)).toBeNull();
  });

  test('enforces the entity-kind prefix and canonical UUID form', () => {
    for (const parse of [
      () => parseConversationId('term_0190f5fe-7c00-7a00-8000-000000000001'),
      () => parseTerminalId('term_0190F5FE-7C00-7A00-8000-000000000001'),
      () => parseTerminalId('term_{0190f5fe-7c00-7a00-8000-000000000001}'),
    ]) {
      let error: unknown;
      try {
        parse();
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof InvalidEntityIdError).toBe(true);
    }
  });

  test('session target comparison includes the entity namespace', () => {
    const conversationId = 'conv_0190f5fe-7c00-7a00-8000-000000000001';
    const terminalId = 'term_0190f5fe-7c00-7a00-8000-000000000001';
    expect(isSameSessionTarget(conversationTarget(conversationId), conversationTarget(conversationId))).toBe(true);
    expect(isSameSessionTarget(conversationTarget(conversationId), terminalTarget(terminalId))).toBe(false);
  });

  test('validates newly registered durable file and document entity ids', () => {
    expect(parseFigureId('figure_0190f5fe-7c00-7a00-8000-000000000001').startsWith('figure_')).toBe(true);
    expect(
      parsePublicAgentAuditEntryId('audit_0190f5fe-7c00-7a00-8000-000000000002').startsWith('audit_')
    ).toBe(true);
    expect(
      parseCompanionEvolutionFeedbackId('evf_0190f5fe-7c00-7a00-8000-000000000003').startsWith('evf_')
    ).toBe(true);
    expect(
      parsePresetTagId('presettag_0190f5fe-7c00-7a00-8000-000000000001').startsWith('presettag_')
    ).toBe(true);
    expect(parseWorkshopNodeId('wsn_0190f5fe-7c00-7a00-8000-000000000002').startsWith('wsn_')).toBe(true);
    expect(parseWorkshopEdgeId('wse_0190f5fe-7c00-7a00-8000-000000000003').startsWith('wse_')).toBe(true);
  });
});
