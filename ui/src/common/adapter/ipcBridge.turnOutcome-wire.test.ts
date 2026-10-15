import { expect, test } from 'bun:test';
import { fromApiResponseMessage, type IResponseMessage } from './ipcBridge';
import { isTurnReadErrorPresentation, transformMessage } from '../chat/chatLib';

test('a host result-read notice crosses the real IPC identity mapper as a public read error', () => {
  const wire = { conversation_id: '01a13e31-57e4-74b1-8f38-19a49e07210c',
    turn_id: '01a13e31-5dea-70f3-9cb5-32231d832a02', msg_id: '01a13e31-5dea-70f3-9cb5-323f5b5ee88f',
    type: 'tips', stream_complete: true, data: { type: 'error', query_error: true,
      content: 'The turn result could not be displayed. Reopen this conversation to retry.' } };
  const mapped = fromApiResponseMessage(wire as IResponseMessage);
  expect(isTurnReadErrorPresentation(mapped)).toBe(true);
  expect(transformMessage(mapped)).toMatchObject({ type: 'tips', turn_id: wire.turn_id,
    msg_id: wire.msg_id, content: { type: 'error', query_error: true } });
  expect(() => fromApiResponseMessage({ ...wire, msg_id: 'read-error:invalid' } as IResponseMessage)).toThrow();
});
