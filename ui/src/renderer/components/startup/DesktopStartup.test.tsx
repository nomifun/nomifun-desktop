import { afterEach, beforeAll, beforeEach, expect, mock, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { DesktopStartupIO, DesktopStartupSnapshot } from '@/common/adapter/tauriStartup';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import DesktopStartup from './DesktopStartup';
const locale=createInstance();
const fixture=(revision:number,stage:DesktopStartupSnapshot['stage']='preparing',phase:DesktopStartupSnapshot['preparation']['phase']='checking',extra:Partial<DesktopStartupSnapshot['preparation']>={}):DesktopStartupSnapshot=>({revision,stage,preparation:{tool:'node',version:'22.22.0',revision,phase,received_bytes:0,total_bytes:null,error:null,...extra}});
const deferred=<T,>()=>{let resolve!:(value:T)=>void;let reject!:(reason:Error)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
let receive:((snapshot:DesktopStartupSnapshot)=>void)|undefined;
const load=mock(async()=>fixture(0));
const retry=mock(async()=>fixture(6));
const quit=mock(async()=>{});
const unlisten=mock(()=>{});
const subscribe=mock(async(callback:(snapshot:DesktopStartupSnapshot)=>void):Promise<()=>void>=>{receive=callback;return unlisten;});
const io:DesktopStartupIO={snapshot:load,retry,quit,subscribe};
beforeAll(async()=>{await locale.init({lng:'en-US',resources:{'en-US':{translation:{common}}}});});
beforeEach(()=>{receive=undefined;load.mockReset().mockImplementation(async()=>fixture(0));retry.mockReset().mockImplementation(async()=>fixture(6));quit.mockReset().mockImplementation(async()=>{});unlisten.mockReset();subscribe.mockReset().mockImplementation(async(callback)=>{receive=callback;return unlisten;});});
afterEach(cleanup);
const mounted=()=>render(<I18nextProvider i18n={locale}><DesktopStartup io={io}/></I18nextProvider>);
const event=(next:DesktopStartupSnapshot)=>act(()=>receive?.(next));
const flush=()=>act(async()=>{});

test('a newer actual progress event wins over a delayed initial snapshot; unknown totals never show a percentage',async()=>{
  const pending=deferred<DesktopStartupSnapshot>();load.mockImplementationOnce(()=>pending.promise);
  const view=mounted();await flush();expect(subscribe).toHaveBeenCalledTimes(1);expect(load).toHaveBeenCalledTimes(1);
  event(fixture(3,'preparing','downloading',{received_bytes:10*1024*1024}));
  expect(view.getByText(common.startup.downloading)).toBeTruthy();expect(view.getByText('10.5 MB downloaded')).toBeTruthy();
  expect(view.queryByRole('progressbar')).toBeNull();
  expect(view.container.querySelector('.arco-progress')).toBeNull();
  await act(async()=>pending.resolve(fixture(1)));
  expect(view.getByText('10.5 MB downloaded')).toBeTruthy();
  event(fixture(2,'failed','error',{error:{code:'COMPONENT_DOWNLOAD_FAILED',message:'old error',retryable:true}}));
  expect(view.queryByRole('alert')).toBeNull();
});

test('known byte totals display real download progress, then verification removes the completed download bar',async()=>{
  const view=mounted();await flush();
  event(fixture(1,'preparing','downloading',{received_bytes:5*1024*1024,total_bytes:20*1024*1024}));
  expect(view.getByText('5.2 MB / 21.0 MB downloaded')).toBeTruthy();
  expect(view.container.querySelector('.arco-progress')).toBeTruthy();
  expect((view.container.querySelector('.arco-progress-line-inner')as HTMLElement)?.style.width).toBe('25%');
  event(fixture(2,'preparing','verifying',{received_bytes:20*1024*1024,total_bytes:20*1024*1024}));
  expect(view.getByText(common.startup.verifying)).toBeTruthy();
  expect(view.container.querySelector('.arco-progress')).toBeNull();
});

test('first preparation failure can retry once and reach backend startup without stale retry replies or runtime selection',async()=>{
  const pending=deferred<DesktopStartupSnapshot>();retry.mockImplementationOnce(()=>pending.promise);
  const view=mounted();await flush();
  event(fixture(5,'failed','error',{error:{code:'COMPONENT_DOWNLOAD_FAILED',message:'offline',retryable:true}}));
  expect(view.getByRole('alert')).toBeTruthy();
  const button=view.getByRole('button',{name:common.retry});fireEvent.click(button);fireEvent.click(button);
  expect(retry).toHaveBeenCalledTimes(1);
  event(fixture(6));event(fixture(7,'preparing','downloading',{received_bytes:8*1024*1024,total_bytes:20*1024*1024}));
  event(fixture(8,'starting_backend','ready'));
  await act(async()=>pending.resolve(fixture(6)));
  expect(view.getByText(common.startup.starting)).toBeTruthy();expect(view.queryByRole('alert')).toBeNull();
  expect(view.queryByRole('button',{name:common.retry})).toBeNull();expect(view.queryByRole('combobox')).toBeNull();
  event(fixture(9,'ready','ready'));
  expect(view.queryByRole('button',{name:common.startup.quit})).toBeNull();
});

test('nonretryable verification and cleanup failures do not offer a runtime replacement or retry',async()=>{
  const view=mounted();await flush();
  event(fixture(4,'failed','error',{error:{code:'COMPONENT_VERIFICATION_FAILED',message:'verification failed',retryable:false}}));
  expect(view.queryByRole('button',{name:common.retry})).toBeNull();
  event(fixture(5,'failed','error',{error:{code:'COMPONENT_CLEANUP_UNPROVEN',message:'cleanup unproven',retryable:false}}));
  expect(view.queryByRole('button',{name:common.retry})).toBeNull();
  expect(view.queryByRole('combobox')).toBeNull();
  fireEvent.click(view.getByRole('button',{name:common.startup.quit}));await flush();expect(quit).toHaveBeenCalledTimes(1);
});

test('quit during download remains visible and disabled while native cleanup runs; old events and late errors cannot revive it',async()=>{
  const pending=deferred<void>();quit.mockImplementationOnce(()=>pending.promise);
  const view=mounted();await flush();
  event(fixture(3,'preparing','downloading',{received_bytes:1024}));
  fireEvent.click(view.getByRole('button',{name:common.startup.quit}));
  event(fixture(4,'quitting','downloading',{received_bytes:2048}));
  expect(view.getByText(common.startup.quitting)).toBeTruthy();
  expect((view.getByRole('button',{name:common.startup.quit})as HTMLButtonElement).disabled).toBe(true);
  event(fixture(3));expect(view.getByText(common.startup.quitting)).toBeTruthy();
  view.unmount();expect(unlisten).toHaveBeenCalledTimes(1);
  await act(async()=>pending.reject(new Error('late native close')));
});

test('unmount before subscription finishes disposes the listener and never requests a startup snapshot',async()=>{
  const pending=deferred<()=>void>();subscribe.mockImplementationOnce(()=>pending.promise);
  const view=mounted();view.unmount();await act(async()=>pending.resolve(unlisten));
  expect(unlisten).toHaveBeenCalledTimes(1);expect(load).not.toHaveBeenCalled();
});


test('a rejected older snapshot does not claim connection failure after live native progress',async()=>{
  const pending=deferred<DesktopStartupSnapshot>();load.mockImplementationOnce(()=>pending.promise);
  const view=mounted();await flush();event(fixture(3,'preparing','downloading',{received_bytes:1000}));
  await act(async()=>pending.reject(new Error('late snapshot delivery failed')));
  expect(view.queryByRole('alert')).toBeNull();expect(view.getByText(common.startup.downloading)).toBeTruthy();
});

test('a lost retry reply does not replace newer native progress with a connection error',async()=>{
  const pending=deferred<DesktopStartupSnapshot>();retry.mockImplementationOnce(()=>pending.promise);
  const view=mounted();await flush();event(fixture(5,'failed','error',{error:{code:'COMPONENT_DOWNLOAD_FAILED',message:'offline',retryable:true}}));
  fireEvent.click(view.getByRole('button',{name:common.retry}));event(fixture(6,'preparing','downloading',{received_bytes:1000}));
  await act(async()=>pending.reject(new Error('native reply was lost')));
  expect(view.queryByRole('alert')).toBeNull();expect(view.getByText(common.startup.downloading)).toBeTruthy();
});
