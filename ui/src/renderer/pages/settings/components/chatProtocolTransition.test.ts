import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { transitionChatProtocolParams, type ChatProtocolTransitionResult } from './chatProtocolTransition';

const fixture: { version: number; cases: Array<{ name: string; source: string; target: string;
  params: Record<string, unknown>; expected: ChatProtocolTransitionResult }> } = JSON.parse(readFileSync(
    new URL('../../../../../../crates/backend/nomifun-model-invoke/tests/fixtures/chat_protocol_transitions.json', import.meta.url), 'utf8',
  ));
for (const item of fixture.cases) test(`shared protocol transition: ${item.name}`, () => {
  const original = structuredClone(item.params);
  const sorted = (result: ChatProtocolTransitionResult) => ({ ...result,
    issues: [...result.issues].sort((left, right) => `${left.parameter}:${left.kind}`.localeCompare(`${right.parameter}:${right.kind}`)),
  });
  expect(sorted(transitionChatProtocolParams(item.params, item.source, item.target))).toEqual(sorted(item.expected));
  expect(item.params).toEqual(original);
});
