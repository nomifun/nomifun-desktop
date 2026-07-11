import { describe, expect, it } from 'vitest';
import { initialMemoryPanelState, memoryPanelReducer } from './memoryPanelProtocol';

describe('memoryPanelReducer', () => {
  it('ignores stale close completion after a newer open', () => {
    const first = memoryPanelReducer(initialMemoryPanelState, { type: 'begin', requestId: 'r1', ownerCompanionId: 'a' });
    const newer = memoryPanelReducer(first, { type: 'begin', requestId: 'r2', ownerCompanionId: 'b' });
    expect(memoryPanelReducer(newer, { type: 'closed', requestId: 'r1' })).toEqual(newer);
  });

  it('accepts blur only after the panel is open', () => {
    const preparing = memoryPanelReducer(initialMemoryPanelState, { type: 'begin', requestId: 'r1', ownerCompanionId: 'a' });
    expect(memoryPanelReducer(preparing, { type: 'request-close', requestId: 'r1', reason: 'blur' })).toEqual(preparing);
    const open = memoryPanelReducer(preparing, { type: 'opened', requestId: 'r1' });
    expect(memoryPanelReducer(open, { type: 'request-close', requestId: 'r1', reason: 'blur' }).phase).toBe('closing');
  });

  it('records Escape as the close reason that restores focus', () => {
    const preparing = memoryPanelReducer(initialMemoryPanelState, { type: 'begin', requestId: 'r1', ownerCompanionId: 'a' });
    const open = memoryPanelReducer(preparing, { type: 'opened', requestId: 'r1' });
    expect(memoryPanelReducer(open, { type: 'request-close', requestId: 'r1', reason: 'escape' }).closeReason).toBe('escape');
  });

  it('ignores duplicate close requests and stale opened events', () => {
    const preparing = memoryPanelReducer(initialMemoryPanelState, { type: 'begin', requestId: 'r2', ownerCompanionId: 'b' });
    expect(memoryPanelReducer(preparing, { type: 'opened', requestId: 'r1' })).toEqual(preparing);
    const open = memoryPanelReducer(preparing, { type: 'opened', requestId: 'r2' });
    const closing = memoryPanelReducer(open, { type: 'request-close', requestId: 'r2', reason: 'toggle' });
    expect(memoryPanelReducer(closing, { type: 'request-close', requestId: 'r2', reason: 'blur' })).toEqual(closing);
  });
});
