import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import type { ReactNode } from 'react';
import { agentPlatform, mode, conversation } from '@/common/adapter/ipcBridge';
import { parseProviderId } from '@/common/types/ids';
import { AGENT_CHAT_MODEL_TASK, CHAT_ROUTE_RECORD_SCHEMA, asAgentPresetId, asCapabilityId, asDigestHex, asResolvedSnapshotId,
  cloneDraft, createEmptyAgentPresetDocument, type AgentPresetEditorResponse, type AgentPresetLibraryResponse,
  type ChatRouteRecord } from '@/common/types/agentPlatform';
import * as auth from '@/renderer/hooks/context/AuthContext';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import { useAgentSettingsController } from './useAgentSettingsController';

const i18n=createInstance();
await i18n.use(initReactI18next).init({lng:'en-US',resources:{'en-US':{translation:{agentSettings:en}}},interpolation:{escapeValue:false}});
const restorers:Array<()=>void>=[];
const watch=<T extends {mockRestore:()=>void}>(spy:T):T=>{restorers.push(()=>spy.mockRestore());return spy;};
beforeEach(()=>{localStorage.clear();setBrowserStorageGeneration('01900000-0000-7000-8000-000000000001');});
afterEach(()=>{cleanup();restorers.splice(0).forEach(restore=>restore());});
const firstId=asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000501');
const secondId=asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000502');
const providerId=parseProviderId('0190f5fe-7c00-7a00-8000-000000000503');
const record=(model='original-model'):ChatRouteRecord=>({schema:CHAT_ROUTE_RECORD_SCHEMA,task:AGENT_CHAT_MODEL_TASK,
  primary:{model_route_id:`route-${model}`,model_route_revision:1,provider_id:providerId,model,protocol:'openai_chat',
    connection_config_ref:'config',credential_ref:'credential',config_revision_digest:asDigestHex('a'.repeat(64)),features:['text_input','text_output','tool_calls']},failovers:[]});
function editor(id=firstId,revisionNumber=1,model:string|null='original-model'):AgentPresetEditorResponse {
  const reference={preset_id:id,revision:revisionNumber,revision_digest:asDigestHex(String(revisionNumber).repeat(64))};
  const document=createEmptyAgentPresetDocument();
  if(model){const selected=record(model);document.model_route_refs[AGENT_CHAT_MODEL_TASK]=selected.primary.model_route_id;document.chat_route_records[AGENT_CHAT_MODEL_TASK]=selected;}
  return {preset:{preset_id:id,source:'user',display_name:id===firstId?'First Agent':'Second Agent',bound_target_count:0,current_stable_revision:reference},
    draft:{preset_id:id,display_name:id===firstId?'First Agent':'Second Agent',current_revision:reference,document},
    revision:{reference,document,created_by:'owner',created_at_ms:revisionNumber}};
}
function deferred<T>(){let resolve!:(value:T)=>void;const promise=new Promise<T>(complete=>{resolve=complete;});return{promise,resolve};}
const pause=async()=>act(async()=>{await new Promise(resolve=>setTimeout(resolve,160));});

async function mount(){
  watch(spyOn(auth,'useAuth').mockReturnValue({user:{id:'01900000-0000-7000-8000-000000000071',username:'owner'}} as ReturnType<typeof auth.useAuth>));
  const editors=new Map([[firstId,editor()],[secondId,editor(secondId,1,'another-valid-model')]]);
  const callbacks=new Set<()=>void>();
  watch(spyOn(mode.onProvidersChanged,'on').mockImplementation(callback=>{const notify=()=>callback({provider_id:providerId});callbacks.add(notify);return()=>{callbacks.delete(notify);};}));
  watch(spyOn(conversation.reconnected,'on').mockReturnValue(()=>{}));
  const library=watch(spyOn(agentPlatform.library,'invoke').mockImplementation(async()=>({official_templates:[],official_presets:{},
    user_presets:[...editors.values()].map(value=>value.preset),active_bindings:[],
    fresh_start:{data_generation:6,legacy_data_imported:false,official_template_count:0,user_preset_count:editors.size}} as AgentPresetLibraryResponse)));
  watch(spyOn(agentPlatform.catalog,'invoke').mockResolvedValue({modules:[],capabilities:[],skills:[],mcp_tools:[],roles:[]}));
  const getEditor=watch(spyOn(agentPlatform.getEditor,'invoke').mockImplementation(async({preset_id})=>structuredClone(editors.get(asAgentPresetId(preset_id))!)));
  const save=watch(spyOn(agentPlatform.saveRevision,'invoke'));
  const create=watch(spyOn(agentPlatform.createPreset,'invoke'));
  const remove=watch(spyOn(agentPlatform.deletePreset,'invoke'));
  const wrapper=({children}:{children:ReactNode})=><I18nextProvider i18n={i18n}><SWRConfig value={{provider:()=>new Map()}}>{children}</SWRConfig></I18nextProvider>;
  const hook=renderHook(()=>useAgentSettingsController(),{wrapper});
  await waitFor(()=>expect(hook.result.current.loading).toBe(false));
  await act(async()=>{await hook.result.current.openPreset(editors.get(firstId)!.preset);});
  const notify=async()=>act(async()=>{callbacks.forEach(callback=>callback());});
  const noWrites=()=>{expect(save).not.toHaveBeenCalled();expect(create).not.toHaveBeenCalled();expect(remove).not.toHaveBeenCalled();};
  return {...hook,editors,library,getEditor,save,create,remove,notify,noWrites};
}

describe('provider changes in the Agent workbench',()=>{
  test.each(['stored', 'returned'] as const)('rebases %s local edits over the host current contract without changing saved history',async source=>{
    const page=await mount();
    const saved=editor();
    saved.draft.document.enabled_capabilities=[{capability:{id:asCapabilityId('workspace.process')},action_allowlist:['workspace.process/exec']}];
    saved.revision!.document=structuredClone(saved.draft.document);
    page.editors.set(firstId,saved);
    await act(async()=>{await page.result.current.openPreset(saved.preset);});
    const local=cloneDraft(page.result.current.draft!);
    local.display_name='My unsaved name';local.document.persona='My unsaved persona';
    local.document.enabled_capabilities[0].action_allowlist!.push('workspace.process/unknown');
    if(source==='stored')await act(async()=>{page.result.current.setDraft(local);});
    const current=structuredClone(saved);
    current.draft.document.enabled_capabilities[0].action_allowlist=['workspace.process/start'];
    page.editors.set(firstId,current);
    await act(async()=>{await page.result.current.openPreset(current.preset,source==='returned'?local:undefined);});
    expect(page.result.current.draft?.document.enabled_capabilities[0].action_allowlist).toEqual(['workspace.process/start','workspace.process/unknown']);
    expect(page.result.current.draft?.display_name).toBe('My unsaved name');
    expect(page.result.current.draft?.document.persona).toBe('My unsaved persona');
    expect(page.result.current.draft?.document.chat_route_records).toEqual(current.draft.document.chat_route_records);
    expect(page.result.current.editor?.revision?.document.enabled_capabilities[0].action_allowlist).toEqual(['workspace.process/exec']);
    expect(page.result.current.editor?.draft.document.enabled_capabilities[0].action_allowlist).toEqual(['workspace.process/start']);
    expect(page.result.current.dirty).toBe(true);
    expect(page.result.current.error).toBeNull();
    page.noWrites();
  });

  test('the host current draft is a clean baseline while its immutable Revision retains retired grants',async()=>{
    const page=await mount();
    const current=editor();
    current.revision!.document=structuredClone(current.draft.document);
    current.revision!.document.enabled_capabilities=[{capability:{id:asCapabilityId('workspace.process')},action_allowlist:['workspace.process/exec']}];
    current.draft.document.enabled_capabilities=[{capability:{id:asCapabilityId('workspace.process')},action_allowlist:['workspace.process/start']}];
    page.editors.set(firstId,current);
    await act(async()=>{await page.result.current.openPreset(current.preset);});
    expect(page.result.current.draft?.document).toEqual(current.draft.document);
    expect(page.result.current.editor?.revision?.document).toEqual(current.revision!.document);
    expect(page.result.current.dirty).toBe(false);
    page.noWrites();
  });

  test('saving adopts and clones the canonical returned document instead of marking the submitted draft as saved',async()=>{
    const page=await mount();
    const submitted=cloneDraft(page.result.current.draft!);
    submitted.document.persona='Keep this authored persona';
    submitted.document.instructions='Submitted instructions';
    await act(async()=>{page.result.current.setDraft(submitted);});
    const saved=editor(firstId,2);
    saved.draft.document=structuredClone(submitted.document);
    saved.draft.document.instructions='Canonical saved instructions';
    saved.revision!.document=structuredClone(saved.draft.document);
    page.editors.set(firstId,saved);
    page.save.mockResolvedValueOnce({preset:saved.preset,revision:saved.revision!,
      resolved_snapshot_ref:{snapshot_id:asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000509'),snapshot_digest:asDigestHex('f'.repeat(64))}});
    await act(async()=>{await page.result.current.saveRevision();});
    expect(page.save.mock.calls[0][0].request.draft.document.instructions).toBe('Submitted instructions');
    expect(page.result.current.draft?.document).toEqual(saved.revision!.document);
    expect(page.result.current.editor?.draft.document).toEqual(saved.revision!.document);
    expect(page.result.current.draft?.document.persona).toBe('Keep this authored persona');
    expect(page.result.current.draft?.document.instructions).toBe('Canonical saved instructions');
    expect(page.result.current.draft?.current_revision?.revision).toBe(2);
    expect(page.result.current.dirty).toBe(false);
    saved.revision!.document.instructions='Later mutation';
    expect(page.result.current.draft?.document.instructions).toBe('Canonical saved instructions');
    expect(page.result.current.editor?.revision?.document.instructions).toBe('Canonical saved instructions');
  });

  test('a pristine editor adopts the cleared server model without saving or choosing a replacement',async()=>{
    const page=await mount();
    page.editors.set(firstId,editor(firstId,2,null));
    await page.notify();
    await waitFor(()=>expect(page.result.current.draft?.current_revision?.revision).toBe(2));
    expect(page.result.current.draft?.document.chat_route_records).toEqual({});
    expect(page.result.current.draft?.document.model_route_refs).toEqual({});
    expect(page.result.current.dirty).toBe(false);
    expect(page.result.current.error).toBeNull();
    page.noWrites();
  });

  test('a model-only server revision rebases the latest non-model typing while preserving dirty input',async()=>{
    const page=await mount();
    await act(async()=>{page.result.current.setDraft({...cloneDraft(page.result.current.draft!),display_name:'Unsaved name'});});
    const next=editor(firstId,2,null);page.editors.set(firstId,next);
    const pending=deferred<AgentPresetEditorResponse>();page.getEditor.mockReturnValueOnce(pending.promise);
    await page.notify();
    await waitFor(()=>expect(page.getEditor).toHaveBeenCalledTimes(2));
    await act(async()=>{const draft=cloneDraft(page.result.current.draft!);draft.document.instructions='Typed while refreshing';page.result.current.setDraft(draft);});
    await act(async()=>{pending.resolve(next);});
    await waitFor(()=>expect(page.result.current.draft?.current_revision?.revision).toBe(2));
    expect(page.result.current.draft?.display_name).toBe('Unsaved name');
    expect(page.result.current.draft?.document.instructions).toBe('Typed while refreshing');
    expect(page.result.current.draft?.document.chat_route_records).toEqual({});
    expect(page.result.current.editor?.draft.display_name).toBe('First Agent');
    expect(page.result.current.dirty).toBe(true);
    expect(page.result.current.error).toBeNull();
    page.noWrites();
  });

  test('an explicitly changed model is preserved with a conflict hint instead of clearing or replacing it',async()=>{
    const page=await mount();
    await act(async()=>{const draft=cloneDraft(page.result.current.draft!);draft.document.chat_route_records[AGENT_CHAT_MODEL_TASK]=record('another-valid-model');
      draft.document.model_route_refs[AGENT_CHAT_MODEL_TASK]='route-another-valid-model';page.result.current.setDraft(draft);});
    page.editors.set(firstId,editor(firstId,2,null));
    await page.notify();
    await waitFor(()=>expect(page.result.current.error).toBe(en.errors.conflict));
    expect(page.result.current.draft?.document.chat_route_records[AGENT_CHAT_MODEL_TASK]?.primary.model).toBe('another-valid-model');
    expect(page.result.current.draft?.current_revision?.revision).toBe(1);
    expect(page.result.current.dirty).toBe(true);
    page.noWrites();
  });

  test('unrelated server authoring changes preserve the draft and require a deliberate reload',async()=>{
    const page=await mount();
    await act(async()=>{const draft=cloneDraft(page.result.current.draft!);draft.document.persona='Unsaved persona';page.result.current.setDraft(draft);});
    const next=editor(firstId,2,null);next.draft.document.instructions='Changed in another window';page.editors.set(firstId,next);
    await page.notify();
    await waitFor(()=>expect(page.result.current.error).toBe(en.errors.conflict));
    expect(page.result.current.draft?.document.persona).toBe('Unsaved persona');
    expect(page.result.current.draft?.document.instructions).toBe('');
    expect(page.result.current.draft?.current_revision?.revision).toBe(1);
    page.noWrites();
  });

  test('a late response for the previously selected Agent cannot reopen it',async()=>{
    const page=await mount();
    const pending=deferred<AgentPresetEditorResponse>();page.getEditor.mockReturnValueOnce(pending.promise);
    await page.notify();await waitFor(()=>expect(page.getEditor).toHaveBeenCalledTimes(2));
    await act(async()=>{await page.result.current.openPreset(page.editors.get(secondId)!.preset);});
    await act(async()=>{pending.resolve(editor(firstId,2,null));});
    await pause();
    expect(page.result.current.draft?.preset_id).toBe(secondId);
    expect(page.result.current.draft?.document.chat_route_records[AGENT_CHAT_MODEL_TASK]?.primary.model).toBe('another-valid-model');
    page.noWrites();
  });

  test('an older revision cannot restore a detached model after a newer refresh',async()=>{
    const page=await mount();
    page.editors.set(firstId,editor(firstId,2,null));await page.notify();
    await waitFor(()=>expect(page.result.current.draft?.current_revision?.revision).toBe(2));
    page.editors.set(firstId,editor(firstId,1));await page.notify();
    await waitFor(()=>expect(page.result.current.error).toBe(en.errors.conflict));
    expect(page.result.current.draft?.current_revision?.revision).toBe(2);
    expect(page.result.current.draft?.document.chat_route_records).toEqual({});
    page.noWrites();
  });

  test('closing the workbench ignores an outstanding provider refresh without any writes',async()=>{
    const page=await mount();
    const pending=deferred<AgentPresetEditorResponse>();page.getEditor.mockReturnValueOnce(pending.promise);
    await page.notify();await waitFor(()=>expect(page.getEditor).toHaveBeenCalledTimes(2));
    page.unmount();await act(async()=>{pending.resolve(editor(firstId,2,null));});
    expect(page.result.current.draft?.current_revision?.revision).toBe(1);
    page.noWrites();
  });

  for(const action of ['save','create','delete','open'] as const)test(`provider refresh waits until an active ${action} completes`,async()=>{
    const page=await mount();
    let operation!:Promise<unknown>;
    let complete!:()=>void;
    if(action==='save'){
      const pending=deferred<Awaited<ReturnType<typeof agentPlatform.saveRevision.invoke>>>();page.save.mockReturnValueOnce(pending.promise);
      const saved=editor(firstId,2);const next=editor(firstId,3,null);page.editors.set(firstId,next);
      complete=()=>pending.resolve({preset:saved.preset,revision:saved.revision!,resolved_snapshot_ref:{snapshot_id:asResolvedSnapshotId('0190f5fe-7c00-7a00-8000-000000000509'),snapshot_digest:asDigestHex('f'.repeat(64))}});
      act(()=>{operation=page.result.current.saveRevision();});
    }else if(action==='create'){
      const pending=deferred<AgentPresetEditorResponse>();page.create.mockReturnValueOnce(pending.promise);
      const next=editor(secondId,2,null);page.editors.set(secondId,next);complete=()=>pending.resolve(next);
      act(()=>{operation=page.result.current.createPreset('Second Agent');});
    }else if(action==='delete'){
      const pending=deferred<void>();page.remove.mockReturnValueOnce(pending.promise);complete=()=>pending.resolve();
      const preset=page.editors.get(firstId)!.preset;page.editors.delete(firstId);
      act(()=>{operation=page.result.current.deletePreset(preset);});
    }else{
      const pending=deferred<AgentPresetEditorResponse>();page.getEditor.mockReturnValueOnce(pending.promise);
      const next=editor(secondId,2,null);page.editors.set(secondId,next);complete=()=>pending.resolve(next);
      act(()=>{operation=page.result.current.openPreset(next.preset);});
    }
    await waitFor(()=>expect(page.result.current.busyAction).toBe(action));
    const reads=page.getEditor.mock.calls.length;
    await page.notify();await pause();
    expect(page.getEditor).toHaveBeenCalledTimes(reads);
    await act(async()=>{complete();await operation;});
    await waitFor(()=>expect(page.result.current.busyAction).toBeNull());
    if(action==='delete'){
      await pause();expect(page.result.current.editor).toBeNull();expect(page.getEditor).toHaveBeenCalledTimes(reads);
    }else{
      await waitFor(()=>expect(page.getEditor.mock.calls.length).toBeGreaterThan(reads));
      await waitFor(()=>expect(page.result.current.draft?.document.chat_route_records).toEqual({}));
    }
    expect(page.save).toHaveBeenCalledTimes(action==='save'?1:0);
    expect(page.create).toHaveBeenCalledTimes(action==='create'?1:0);
    expect(page.remove).toHaveBeenCalledTimes(action==='delete'?1:0);
  });
});
