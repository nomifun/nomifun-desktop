import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Alert, Spin } from '@arco-design/web-react';
import styles from './PluginRuntimeProduct.module.css';

/** Every preview document gets a fresh in-frame store. It never receives a production capability. */
export function previewDocument(html: string, token: string): string {
  const bootstrap = `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'"><script>(()=>{const token=${JSON.stringify(token)};const data=new Map();const report=(error)=>parent.postMessage({type:'nomifun-plugin-preview',token,error},'*');Object.defineProperty(window,'nomi',{value:Object.freeze({preview:true,service:Object.freeze({async invoke(){throw new Error("Background work is available after saving this plugin.")}}),storage:Object.freeze({async get(key){return data.has(key)?structuredClone(data.get(key)):null},async set(key,value){data.set(key,structuredClone(value))},async delete(key){data.delete(key)}})}),writable:false});let failed=false;window.addEventListener('error',event=>{failed=true;report(String(event.message||'Preview failed').slice(0,1000))});window.addEventListener('unhandledrejection',event=>{failed=true;report(String(event.reason?.message||event.reason||'Preview failed').slice(0,1000))});window.addEventListener('DOMContentLoaded',()=>setTimeout(()=>{if(!failed)report(null)},500));})();</script>`;
  // Exported releases carry the production bridge before their HTML document.
  // Remove only host-owned bootstrap scripts and establish the temporary API
  // before ANY app code executes; imported previews must never wait for a
  // production capability or overwrite the preview's in-memory storage API.
  const source = html
    .replace(
      /<script\b[^>]*\bdata-nomifun-(?:miniapp-bridge|product-sdk)\s*=[^>]*>[\s\S]*?<\/script\s*>/gi,
      '',
    )
    .replace(/<!doctype[^>]*>/gi, '');
  return `<!doctype html>${bootstrap}${source}`;
}

export default function PluginRuntimeDraftPreview({
  html,
  title,
  onStatus,
}: {
  html: string;
  title: string;
  onStatus?: (error: string | null) => void;
}) {
  const { t } = useTranslation();
  const frame = useRef<HTMLIFrameElement>(null);
  const callback = useRef(onStatus);
  callback.current = onStatus;
  const token = useMemo(() => crypto.randomUUID(), [html]);
  const document = useMemo(() => previewDocument(html, token), [html, token]);
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>(
    'loading',
  );
  useEffect(() => {
    setStatus('loading');
    let received = false;
    const receive = (event: MessageEvent) => {
      if (
        event.source !== frame.current?.contentWindow ||
        event.data?.type !== 'nomifun-plugin-preview' ||
        event.data?.token !== token
      )
        return;
      received = true;
      const error =
        typeof event.data.error === 'string'
          ? event.data.error.slice(0, 1000)
          : null;
      setStatus(error ? 'error' : 'ready');
      callback.current?.(error);
    };
    window.addEventListener('message', receive);
    const timeout = window.setTimeout(() => {
      if (!received) {
        setStatus('error');
        callback.current?.('Preview initialization timed out');
      }
    }, 15000);
    return () => {
      window.removeEventListener('message', receive);
      window.clearTimeout(timeout);
    };
  }, [token]);
  return (
    <>
      {status === 'loading' && (
        <span className={styles.muted} role='status'>
          <Spin size={12} /> {t('pluginRuntime.product.checking')}
        </span>
      )}
      {status === 'error' && (
        <Alert type='warning' content={t('pluginRuntime.product.previewError')} />
      )}
      <iframe
        ref={frame}
        key={token}
        title={title}
        className={styles.frame}
        srcDoc={document}
        sandbox='allow-scripts allow-forms'
        referrerPolicy='no-referrer'
      />
    </>
  );
}
