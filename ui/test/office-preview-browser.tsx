/** Real-engine acceptance harness. All preview calls reach actual loopback Office routes. */
import { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { createInstance } from 'i18next';
import '@arco-design/web-react/es/_util/react-19-adapter';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import common from '../src/renderer/services/i18n/locales/en-US/common.json';
import preview from '../src/renderer/services/i18n/locales/en-US/preview.json';
import type { OfficePreviewDocType } from '../src/common/types/office/preview';
import { initializeAgentBrowserStorageGeneration } from '../src/common/utils/browserStorageKey';

initializeAgentBrowserStorageGeneration(1);
const { default: OfficeArtifactViewer } = await import('../src/renderer/pages/conversation/Preview/components/viewers/OfficeArtifactViewer');
const locale = createInstance();
await locale.use(initReactI18next).init({ lng:'en-US',resources:{'en-US':{translation:{common,preview}}} });
const response = await fetch('/api/office-browser-fixture/info');
if (!response.ok) throw new Error('Start the real Office browser fixture backend before this harness');
const fixture = await response.json() as {ppt:string;word:string;excel:string;workspace:string;engine_version:string;cache_entries:number};
const realFetch = globalThis.fetch.bind(globalThis);
const requests = {start:0,status:0,stop:0};
// Observe only; no response, delay, tool output or successful state is fabricated.
globalThis.fetch = async (input, init) => {
  const url = new URL(typeof input==='string'?input:input instanceof URL?input.href:input.url,location.href);
  for (const key of ['start','status','stop'] as const) if(url.pathname.startsWith(`/api/office-preview/${key}`)) ++requests[key];
  return realFetch(input,init);
};

function Acceptance() {
  const [type,setType] = useState<OfficePreviewDocType>('ppt');
  const [revision,setRevision] = useState(0);
  const [stopped,setStopped] = useState(false);
  const [metrics,setMetrics] = useState({phase:'preparing',firstPaintMs:0,starts:0,polls:0,stops:0,cacheEntries:0,retainedOnUpdate:false,peakFrames:0,policy:'not checked',capability:'not checked'});
  const container = useRef<HTMLDivElement>(null);
  const started = useRef(performance.now());
  const lastArtifact = useRef('');
  const lastRequest = useRef('');
  const expectedUpdate = useRef(false);
  useEffect(() => {
    const inspect = () => {
      const root = container.current?.firstElementChild;
      const frames = Array.from(container.current?.querySelectorAll('iframe') ?? []);
      const visible = frames.find((frame)=>frame.getAttribute('aria-hidden')==='false');
      const busy = root?.getAttribute('aria-busy')==='true';
      if (visible) {lastArtifact.current=visible.src;lastRequest.current=new URL(visible.src).searchParams.get('request_id')??'';}
      setMetrics((previous)=>({...previous,phase:stopped?'stopped':root?.querySelector('[role=alert]')?'error':busy?'processing':visible?'ready':'preparing',
        firstPaintMs:!busy&&visible?Math.round(performance.now()-started.current):previous.firstPaintMs,
        starts:requests.start,polls:requests.status,stops:requests.stop,
        retainedOnUpdate:previous.retainedOnUpdate||(expectedUpdate.current&&busy&&!!visible),peakFrames:Math.max(previous.peakFrames,frames.length)}));
    };
    const observer = new MutationObserver(inspect);
    if(container.current)observer.observe(container.current,{childList:true,subtree:true,attributes:true,attributeFilter:['aria-busy','aria-hidden']});
    inspect();
    return ()=>observer.disconnect();
  },[type,revision,stopped]);
  const select = (next:OfficePreviewDocType) => {started.current=performance.now();expectedUpdate.current=false;setType(next);setStopped(false);setMetrics((previous)=>({...previous,phase:'preparing',firstPaintMs:0}));};
  const update = async () => {
    const result=await fetch('/api/office-browser-fixture/update',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({doc_type:type})});
    if(!result.ok)throw new Error(await result.text());
    expectedUpdate.current=true;started.current=performance.now();setRevision((value)=>value+1);
  };
  const inspectPolicy = async () => {
    const result=await fetch(lastArtifact.current,{method:'HEAD'});
    const next=await fetch('/api/office-browser-fixture/info').then((result)=>result.json()) as typeof fixture;
    const policy=result.headers.get('content-security-policy')??'not present';
    const state=await fetch(`/api/office-preview/status/${lastRequest.current}`).then((result)=>result.json()) as {data?:{error?:{code?:string}}};
    setMetrics((previous)=>({...previous,cacheEntries:next.cache_entries,policy:policy.includes("sandbox allow-scripts")&&policy.includes("connect-src 'none'")?'sandbox + network denied':policy,
      capability:`HTTP ${result.status}; request ${state.data?.error?.code??'active'}`,starts:requests.start,polls:requests.status,stops:requests.stop}));
  };
  return <main style={{height:'100vh',minHeight:600,minWidth:880,display:'flex',flexDirection:'column',padding:16,boxSizing:'border-box',fontFamily:'Arial,sans-serif',background:'#f2f3f5'}}>
    <header style={{display:'flex',alignItems:'center',gap:12,marginBottom:10}}><strong>Actual Office artifact acceptance</strong><span>OfficeCLI {fixture.engine_version}</span><span>Desktop {innerWidth} × {innerHeight}</span></header>
    <nav style={{display:'flex',gap:8,marginBottom:10}}>{(['ppt','word','excel'] as const).map((kind)=><button key={kind} onClick={()=>select(kind)}>{kind.toUpperCase()} fixture</button>)}<button onClick={()=>void update()} disabled={stopped}>Update source</button><button onClick={()=>setStopped(true)}>Stop preview</button><button onClick={()=>void inspectPolicy()} disabled={!lastArtifact.current}>Check capability and policy</button></nav>
    <section ref={container} style={{flex:1,minHeight:0,border:'1px solid #b5b9c0',borderRadius:8,overflow:'hidden',background:'white'}}>{!stopped&&<OfficeArtifactViewer docType={type} file_path={fixture[type]} workspace={fixture.workspace} revision={revision}/>}</section>
    <output style={{display:'block',fontFamily:'monospace',fontSize:13,paddingTop:12,whiteSpace:'pre-wrap'}}>{JSON.stringify(metrics,null,2)}</output>
  </main>;
}
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={locale}><Acceptance/></I18nextProvider>);
