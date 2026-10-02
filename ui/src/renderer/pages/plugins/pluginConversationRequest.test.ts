import { expect, test } from 'bun:test';
import { initialPluginDelivery } from './pluginConversationRequest';
import { launchPluginConversation, readPluginLaunchIntent, consumePluginLaunchIntent } from './pluginConversationLaunch';
import type { NavigateFunction } from 'react-router-dom';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';

const bootstrap = 'Read the existing plugin and wait for my changes.';
const intent = { version: 1 as const, token: 'intent', owner_user_id: 'owner', created_at: Date.now() };

test('unchanged edit/resume bootstrap leaves the conversation available for requirements', () => {
  expect(initialPluginDelivery({ ...intent, plugin_id: 'plugin' }, bootstrap, bootstrap)).toBeUndefined();
  expect(initialPluginDelivery({ ...intent, draft_id: 'draft' }, ` ${bootstrap} `, bootstrap)).toBeUndefined();
});

test('new creation and a concrete edit retain the delivery obligation', () => {
  expect(initialPluginDelivery(intent, 'Create a todo app', '')).toEqual({});
  expect(initialPluginDelivery({ ...intent, plugin_id: 'plugin' }, 'Add a delete button', bootstrap)).toEqual({});
  expect(initialPluginDelivery({ ...intent, requirement: bootstrap, draft_id: 'draft' }, bootstrap, bootstrap)).toEqual({});
});

test('workbench detour retains the edited requirement and attachments on the same intent', async () => {
  setBrowserStorageGeneration('01900000-0000-7000-8000-000000000001');
  const realFetch=globalThis.fetch;
  let destination='';
  const navigate=((path: string) => { destination=path; }) as NavigateFunction;
  globalThis.fetch=(async () => new Response(JSON.stringify({success:true,data:{
    status:'configure_agent',reason:'PLUGIN_MODULE_DISABLED',owner_user_id:'owner',
    selection:{kind:'template',templateKey:'assistant.general'},
  }}),{headers:{'Content-Type':'application/json'}})) as typeof fetch;
  let token='';
  try {
    await launchPluginConversation(navigate,{requirement:'Initial request'});
    token=new URLSearchParams(destination.split('?')[1]).get('pluginIntent')!;
    await launchPluginConversation(navigate,{requirement:'Edited request',files:['/managed/attachment.txt']},token);
    expect(new URLSearchParams(destination.split('?')[1]).get('pluginIntent')).toBe(token);
    expect(readPluginLaunchIntent(token,'owner')).toMatchObject({requirement:'Edited request',files:['/managed/attachment.txt']});
    expect(readPluginLaunchIntent(token,'another-owner')).toBeNull();
  } finally { globalThis.fetch=realFetch; if(token) consumePluginLaunchIntent(token); }
});
