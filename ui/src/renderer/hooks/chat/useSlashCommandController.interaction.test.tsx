import '../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useState } from 'react';
import { matchSlashQuery, useSlashCommandController } from './useSlashCommandController';

afterEach(cleanup);

test('slash query supports exact namespaced IDs but ends at arguments', () => {
  expect(matchSlashQuery('/')).toBe('');
  expect(matchSlashQuery('/skill:acme.review-v2')).toBe('skill:acme.review-v2');
  expect(matchSlashQuery('/skill:acme.review args')).toBeNull();
  expect(matchSlashQuery('/skill:acme.review\n')).toBeNull();
  expect(matchSlashQuery('text /skill:acme.review')).toBeNull();
});

test('selecting a namespaced Skill inserts exact command without executing a UI builtin', () => {
  const executed: string[] = [];
  function Harness() {
    const [input, setInput] = useState('');
    const controller = useSlashCommandController({ input,
      commands: [
        { name: 'copy', description: '', kind: 'builtin', source: 'builtin' },
        { name: 'skill:acme.review', description: 'Review', kind: 'template', source: 'agent', selectionBehavior: 'insert' },
        { name: 'skill:acme.other', description: 'Other', kind: 'template', source: 'agent', selectionBehavior: 'insert' },
      ], onExecuteBuiltin: name => executed.push(name), onSelectTemplate: name => setInput(`/${name} `),
    });
    return <><input aria-label='Message' value={input} onChange={event => setInput(event.target.value)}
      onKeyDown={controller.onKeyDown} />
      {controller.isOpen && controller.filteredCommands.map(command => <span key={command.name}>{command.name}</span>)}
    </>;
  }
  const view = within(render(<Harness />).container);
  const input = view.getByRole('textbox') as HTMLInputElement;
  fireEvent.change(input, { target: { value: '/skill:acme.re' } });
  expect(view.getByText('skill:acme.review')).toBeTruthy();
  expect(view.queryByText('skill:acme.other')).toBeNull();
  fireEvent.keyDown(input, { key: 'Enter' });
  expect(input.value).toBe('/skill:acme.review ');
  expect(executed).toEqual([]);
  expect(view.queryByText('skill:acme.review')).toBeNull();
});
