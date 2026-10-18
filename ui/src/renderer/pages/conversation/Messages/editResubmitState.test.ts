import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
const sendBoxSource = readFileSync(
  new URL('../../../components/chat/SendBox/index.tsx', import.meta.url),
  'utf8'
);
const nomiSendBoxSource = readFileSync(
  new URL('../platforms/nomi/NomiSendBox.tsx', import.meta.url),
  'utf8'
);

describe('edit/resubmit local suffix replacement', () => {

  test('exits edit mode after ordinary input is saved and clears only the submitted draft', () => {
    const editSubmitBranch = sendBoxSource.slice(
      sendBoxSource.indexOf('if (editingMsgId && onEditResubmit) {'),
      sendBoxSource.indexOf('// Cancel any pending warmup:')
    );
    const submit = editSubmitBranch.indexOf(
      'onEditResubmit(targetId, targetCreatedAt, finalMessage)'
    );
    const accepted = editSubmitBranch.indexOf('.then(() => {', submit);
    const exitEditMode = editSubmitBranch.indexOf('setEditingMsgId(null);', submit);
    const clearInput = editSubmitBranch.indexOf("if (latestInputRef.current === finalMessage) setInputRef.current('');", submit);

    expect(submit).toBeGreaterThan(-1);
    expect(accepted).toBeGreaterThan(submit);
    expect(exitEditMode).toBeGreaterThan(accepted);
    expect(clearInput).toBeGreaterThan(accepted);

    const nomiHandler = nomiSendBoxSource.slice(
      nomiSendBoxSource.indexOf('const handleEditResubmit = async'),
      nomiSendBoxSource.indexOf('const handleEditQueuedCommand')
    );
    expect(nomiHandler.includes('await onSendHandler(message);')).toBe(true);
    expect(nomiHandler.includes('removeMessagesByLocalIds')).toBe(false);
    expect(nomiHandler.includes('editResubmit.invoke')).toBe(false);
  });
});
