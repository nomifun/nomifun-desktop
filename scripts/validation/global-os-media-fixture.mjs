#!/usr/bin/env node
// A loopback-only device/permission fixture for the visible native Browser.
// It never navigates UI, grants permission, records media or creates an Agent.
import assert from 'node:assert/strict';
import { randomBytes, randomUUID } from 'node:crypto';
import { createServer } from 'node:http';
import { mkdir, realpath, writeFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const allowedErrors = new Set(['NotAllowedError', 'NotFoundError', 'NotReadableError', 'OverconstrainedError',
  'AbortError', 'TypeError', 'SecurityError', 'InvalidStateError', 'NotSupportedError', 'TimeoutError', 'UnknownError']);
const allowedReasons = new Set(['complete', 'insecure_context', 'capture_unavailable', 'audio_context_unavailable',
  'page_not_visible', 'video_data_timeout', 'audio_clock_timeout', 'capture_failed', 'cleanup_incomplete']);
// Only generic constraint names are useful for diagnosis. Device/group IDs,
// labels, error messages and arbitrary constraint strings are never retained.
const allowedConstraints = new Set(['', 'unknown', 'width', 'height', 'aspectRatio', 'frameRate', 'facingMode', 'resizeMode',
  'sampleRate', 'sampleSize', 'echoCancellation', 'autoGainControl', 'noiseSuppression', 'latency', 'channelCount', 'volume']);
const captureModes = ['audio_video', 'audio', 'video'];
const boolFields = ['secure_context', 'tracks_live_before_stop', 'video_loaded_data', 'video_frame_callback_available',
  'video_frame_callback_received', 'audio_graph_connected', 'audio_context_running', 'audio_context_time_advanced',
  'all_tracks_ended', 'audio_context_closed'];
const integerFields = ['audio_tracks', 'video_tracks', 'total_tracks', 'video_width', 'video_height',
  'audio_sample_rate', 'analyser_fft_size', 'elapsed_ms'];

export function sanitizeMediaResult(value) {
  assert(value && typeof value === 'object' && !Array.isArray(value), 'Expected bounded metadata object');
  const legacy = value.schema === 'nomifun.local-media-attempt.v1';
  assert(legacy || value.schema === 'nomifun.local-media-attempt.v2', 'Unsupported media evidence schema');
  const permitted = new Set(['schema', 'status', 'reason', 'error_name', 'attempt', ...boolFields, ...integerFields,
    ...(legacy ? [] : ['capture_mode', 'error_constraint'])]);
  assert(Object.keys(value).every(key => permitted.has(key)), 'Unexpected fields are forbidden in media evidence');
  assert(['passed', 'failed'].includes(value.status));
  assert(Number.isInteger(value.attempt) && value.attempt >= 1 && value.attempt <= 32);
  assert(allowedReasons.has(value.reason));
  assert(value.error_name === undefined || allowedErrors.has(value.error_name), 'Unknown error names are forbidden');
  const clean = { schema: value.schema, attempt: value.attempt, status: value.status, reason: value.reason,
    error_name: value.error_name };
  if (!legacy) {
    assert(captureModes.includes(value.capture_mode), 'Unknown capture mode');
    assert(allowedConstraints.has(value.error_constraint), 'Unknown or private error constraint');
    clean.capture_mode = value.capture_mode;clean.error_constraint = value.error_constraint;
  }
  for (const name of boolFields) { assert.equal(typeof value[name], 'boolean', `Invalid boolean metadata ${name}`); clean[name] = value[name]; }
  for (const name of integerFields) {
    assert(Number.isInteger(value[name]) && value[name] >= 0 && value[name] <= 10_000_000, `Invalid integer metadata ${name}`);
    clean[name] = value[name];
  }
  const mode = legacy ? 'audio_video' : clean.capture_mode, wantsAudio = mode !== 'video', wantsVideo = mode !== 'audio';
  const audioVerified = wantsAudio ? clean.audio_tracks === 1 && clean.audio_graph_connected && clean.audio_context_running
    && clean.audio_context_time_advanced && clean.audio_sample_rate > 0 && clean.analyser_fft_size > 0
    : clean.audio_tracks === 0 && !clean.audio_graph_connected && !clean.audio_context_running && !clean.audio_context_time_advanced
      && clean.audio_sample_rate === 0 && clean.analyser_fft_size === 0;
  const videoVerified = wantsVideo ? clean.video_tracks === 1 && clean.video_loaded_data && clean.video_width > 0 && clean.video_height > 0
    : clean.video_tracks === 0 && !clean.video_loaded_data && clean.video_width === 0 && clean.video_height === 0
      && !clean.video_frame_callback_available && !clean.video_frame_callback_received;
  const verified = clean.secure_context && clean.total_tracks === Number(wantsAudio) + Number(wantsVideo)
    && clean.tracks_live_before_stop && audioVerified && videoVerified && clean.all_tracks_ended && clean.audio_context_closed;
  assert(clean.status !== 'passed' || (verified && clean.reason === 'complete' && !clean.error_name && (legacy || clean.error_constraint === '')), 'A media PASS requires all real capture and cleanup conditions');
  return clean;
}

export function buildMediaFixturePage({ pageToken, cspNonce }) {
  assert(/^[a-f0-9]{48}$/.test(pageToken) && /^[a-f0-9]{48}$/.test(cspNonce));
  return `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>NomiFun 本机媒体许可验收</title>
<style nonce="${cspNonce}">html{font:16px system-ui;color:#20242b;background:#f3f5f6}body{max-width:760px;margin:48px auto;padding:0 28px}h1{font-size:24px}p{line-height:1.7}button{padding:12px 18px;font:inherit}pre{white-space:pre-wrap;padding:16px;background:white;border:1px solid #ccd2d7;border-radius:8px}</style></head><body>
<h1>NomiFun 本机媒体许可验收</h1><p>仅在这个可见原生 Browser 页面中点击下方按钮。按实际请求处理网站许可和 macOS 原生摄像头、麦克风许可。页面不显示、录制、保存或上传音视频内容，只向本机回报轨道状态、视频尺寸与音频处理图状态，然后立即释放全部设备。</p>
<button id="start-both" type="button">摄像头与麦克风</button> <button id="start-audio" type="button">仅麦克风</button> <button id="start-video" type="button">仅摄像头</button><p id="status" role="status">等待点击。每次只运行所选模式；每次结果独立保留，拒绝或失败不会被后续重试覆盖。</p><pre id="result">尚未使用设备。</pre>
<script nonce="${cspNonce}">
const modeButtons=[['start-both','audio_video'],['start-audio','audio'],['start-video','video']];
const buttons=modeButtons.map(([id])=>document.getElementById(id)),status=document.getElementById('status'),output=document.getElementById('result');
const constraintNames=new Set(${JSON.stringify([...allowedConstraints].filter(name => name !== '' && name !== 'unknown'))});
let attempt=0,activeStream,running=false;
const stopLiveTracks=()=>{if(activeStream)for(const track of activeStream.getTracks())track.stop();};
window.addEventListener('pagehide',stopLiveTracks);
document.addEventListener('visibilitychange',()=>{if(document.hidden)stopLiveTracks();});
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function loadedVideo(video){if(video.readyState>=2)return;await new Promise((resolve,reject)=>{const done=error=>{clearTimeout(timer);video.removeEventListener('loadeddata',ready);video.removeEventListener('error',failed);error?reject(error):resolve();};const ready=()=>done();const failed=()=>done(new DOMException('Video loading failed','NotReadableError'));const timer=setTimeout(()=>done(new DOMException('Video data deadline elapsed','TimeoutError')),8000);video.addEventListener('loadeddata',ready,{once:true});video.addEventListener('error',failed,{once:true});});}
async function boundedPlayback(video){let timer;try{await Promise.race([Promise.all([video.play(),loadedVideo(video)]),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new DOMException('Video playback deadline elapsed','TimeoutError')),8000);})]);}finally{clearTimeout(timer);}}
async function capture(captureMode){
  if(running||attempt>=32)return;running=true;for(const button of buttons)button.disabled=true;
  status.textContent='正在等待所选模式的真实网站与系统许可；请完成原生交互。';
  const wantsAudio=captureMode!=='video',wantsVideo=captureMode!=='audio';
  const started=performance.now();let stream,video,context,source,analyser,audioActivation;
  const result={schema:'nomifun.local-media-attempt.v2',capture_mode:captureMode,error_constraint:'',attempt:++attempt,status:'failed',reason:'capture_failed',
    secure_context:window.isSecureContext,tracks_live_before_stop:false,video_loaded_data:false,video_frame_callback_available:false,video_frame_callback_received:false,
    audio_graph_connected:false,audio_context_running:false,audio_context_time_advanced:false,all_tracks_ended:false,audio_context_closed:false,
    audio_tracks:0,video_tracks:0,total_tracks:0,video_width:0,video_height:0,audio_sample_rate:0,analyser_fft_size:0,elapsed_ms:0};
  try{
    if(!window.isSecureContext){result.reason='insecure_context';throw new DOMException('Secure context required','SecurityError');}
    if(!navigator.mediaDevices?.getUserMedia){result.reason='capture_unavailable';throw new DOMException('Capture unavailable','NotSupportedError');}
    if(wantsAudio){
      if(!window.AudioContext){result.reason='audio_context_unavailable';throw new DOMException('Audio graph unavailable','NotSupportedError');}
      // Resume while the real gesture is active, before awaiting native consent.
      context=new AudioContext();audioActivation=context.resume().then(()=>true,()=>false);
    }
    // No timeout abandons this request: a stream obtained after native consent
    // still enters the exact finally cleanup below. No device labels/IDs read.
    stream=await navigator.mediaDevices.getUserMedia({audio:wantsAudio,video:wantsVideo});activeStream=stream;
    if(document.hidden){result.reason='page_not_visible';throw new DOMException('Page is not visible','AbortError');}
    result.audio_tracks=stream.getAudioTracks().length;result.video_tracks=stream.getVideoTracks().length;result.total_tracks=stream.getTracks().length;
    result.tracks_live_before_stop=stream.getTracks().every(track=>track.readyState==='live');
    let audioStart;
    if(wantsAudio){
      source=context.createMediaStreamSource(stream);analyser=context.createAnalyser();analyser.fftSize=1024;source.connect(analyser);
      // Silent graph only: no output connection and no sample reads.
      result.audio_graph_connected=true;await Promise.race([audioActivation,sleep(3000)]);result.audio_context_running=context.state==='running';result.audio_sample_rate=Math.round(context.sampleRate);result.analyser_fft_size=analyser.fftSize;
      if(!result.audio_context_running){result.reason='audio_clock_timeout';throw new DOMException('Audio graph did not start','TimeoutError');}
      audioStart=context.currentTime;
    }
    if(wantsVideo){
      // Detached decode metadata only: no rendered video or pixel extraction.
      video=document.createElement('video');video.muted=true;video.playsInline=true;video.srcObject=stream;
      await boundedPlayback(video);
      result.video_loaded_data=video.readyState>=2;result.video_width=video.videoWidth;result.video_height=video.videoHeight;
      result.video_frame_callback_available=typeof video.requestVideoFrameCallback==='function';
      if(result.video_frame_callback_available){await new Promise(resolve=>{let callback;const timer=setTimeout(()=>{if(callback!==undefined)video.cancelVideoFrameCallback?.(callback);resolve();},1200);callback=video.requestVideoFrameCallback(()=>{clearTimeout(timer);result.video_frame_callback_received=true;resolve();});});}
    }
    if(wantsAudio){await sleep(180);result.audio_context_time_advanced=context.currentTime>audioStart+0.03;}
    result.tracks_live_before_stop=result.tracks_live_before_stop&&stream.getTracks().every(track=>track.readyState==='live');
    if(wantsAudio&&!result.audio_context_time_advanced){result.reason='audio_clock_timeout';throw new DOMException('Audio graph clock did not advance','TimeoutError');}
    result.reason='complete';result.status='passed';
  }catch(error){result.status='failed';result.error_name=['NotAllowedError','NotFoundError','NotReadableError','OverconstrainedError','AbortError','TypeError','SecurityError','InvalidStateError','NotSupportedError','TimeoutError'].includes(error?.name)?error.name:'UnknownError';if(error?.name==='OverconstrainedError')result.error_constraint=error.constraint===''?'':constraintNames.has(error.constraint)?error.constraint:'unknown';if(error?.name==='TimeoutError'&&result.reason==='capture_failed')result.reason='video_data_timeout';}
  finally{
    if(stream)for(const track of stream.getTracks())track.stop();activeStream=undefined;
    result.all_tracks_ended=stream?stream.getTracks().every(track=>track.readyState==='ended'):true;
    if(video){video.pause();video.srcObject=null;}source?.disconnect();analyser?.disconnect();
    if(context){try{await Promise.race([context.close(),sleep(2000)]);result.audio_context_closed=context.state==='closed';}catch{result.audio_context_closed=false;}}else result.audio_context_closed=true;
    if(!result.all_tracks_ended||!result.audio_context_closed){result.status='failed';result.reason='cleanup_incomplete';}
    result.elapsed_ms=Math.max(0,Math.round(performance.now()-started));
    const audioVerified=wantsAudio?result.audio_tracks===1&&result.audio_graph_connected&&result.audio_context_running&&result.audio_context_time_advanced&&result.audio_sample_rate>0&&result.analyser_fft_size>0:result.audio_tracks===0;
    const videoVerified=wantsVideo?result.video_tracks===1&&result.video_loaded_data&&result.video_width>0&&result.video_height>0:result.video_tracks===0;
    if(result.status==='passed'&&!(result.secure_context&&result.total_tracks===Number(wantsAudio)+Number(wantsVideo)&&result.tracks_live_before_stop&&audioVerified&&videoVerified)){result.status='failed';result.reason='capture_failed';}
    output.textContent=JSON.stringify(result,null,2);status.textContent=result.status==='passed'?'真实采集与释放已核验；正在提交纯状态元数据。':'本次失败已保留；设备已请求释放，不把拒绝或未就绪当成成功。';
    try{const response=await fetch('/${pageToken}/result',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(result)});if(!response.ok)throw new Error('Metadata rejected');status.textContent+=' 元数据已保存到本机验收目录。';}catch{status.textContent+=' 元数据未能提交；这不代表服务器已收到结果。';}
    running=false;for(const button of buttons)button.disabled=attempt>=32;
  }
}
for(const [id,captureMode] of modeButtons)document.getElementById(id).addEventListener('click',()=>capture(captureMode));
</script></body></html>`;
}

export async function startGlobalOsMediaFixture({ reportDirectory }) {
  assert.equal(process.platform, 'darwin', 'This native media fixture is intended for the actual macOS App');
  assert(isAbsolute(reportDirectory), 'An absolute isolated evidence directory is required');
  await mkdir(reportDirectory, { recursive: true, mode: 0o700 });
  assert.equal(await realpath(reportDirectory), reportDirectory, 'Evidence path must be canonical');
  const pageToken = randomBytes(24).toString('hex'), cspNonce = randomBytes(24).toString('hex'), runId = randomUUID();
  const html = buildMediaFixturePage({ pageToken, cspNonce });
  const csp = `default-src 'none'; script-src 'nonce-${cspNonce}'; style-src 'nonce-${cspNonce}'; connect-src 'self'; media-src 'self' blob:; img-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`;
  let origin, attemptCount = 0;
  const reportFiles = [], pendingRequests = new Set();
  const handleRequest = async (request, response) => {
    response.setHeader('cache-control', 'no-store');response.setHeader('x-content-type-options', 'nosniff');response.setHeader('referrer-policy', 'no-referrer');
    try {
      if (request.url === `/${pageToken}/` && request.method === 'GET') {
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'content-security-policy': csp,
          'permissions-policy': 'camera=(self), microphone=(self), display-capture=(), geolocation=()', 'cross-origin-opener-policy': 'same-origin' });
        response.end(html);return;
      }
      if (request.url !== `/${pageToken}/result` || request.method !== 'POST') { response.writeHead(404);response.end();return; }
      assert.equal(request.headers.origin, origin, 'Only the nonce page origin may submit metadata');
      assert.equal(request.headers['content-type'], 'application/json');
      assert(attemptCount < 32, 'Bounded media attempt count exceeded');
      let size = 0;const buffers = [];
      for await (const bytes of request) { size += bytes.length;assert(size <= 16_384, 'Only small state metadata is accepted');buffers.push(bytes); }
      const submitted = JSON.parse(Buffer.concat(buffers).toString('utf8'));
      assert.equal(submitted.schema, 'nomifun.local-media-attempt.v2', 'New submissions must identify their capture mode');
      const metadata = sanitizeMediaResult(submitted);
      // Reading a body yields to other requests. Recheck immediately before
      // reserving the slot so concurrent submissions cannot exceed the bound.
      assert(attemptCount < 32, 'Bounded media attempt count exceeded');
      const reportPath = join(reportDirectory, `media-attempt-${++attemptCount}-${runId}.json`);
      await writeFile(reportPath, `${JSON.stringify({ ...metadata, received_at: new Date().toISOString(),
        entrypoint: 'nonce-protected loopback media fixture', scope: 'Self-reported client device/OS permission-chain metadata; visible native App identity, user interaction and permission UI require separate evidence',
        privacy: { video_or_audio_content_retained: false, samples_read_or_retained: false, device_ids_or_labels_read: false } }, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
      reportFiles.push(reportPath);response.writeHead(200, { 'content-type': 'application/json' });response.end('{"received":true}');
      console.log(JSON.stringify({ status: metadata.status, capture_mode: metadata.capture_mode, error_name: metadata.error_name,
        error_constraint: metadata.error_constraint, report: reportPath }));
    } catch { response.writeHead(400, { 'content-type': 'application/json' });response.end('{"received":false,"reason":"invalid_or_nonprivate_metadata"}'); }
  };
  const server = createServer((request, response) => {
    const pending = handleRequest(request, response);pendingRequests.add(pending);
    void pending.then(() => pendingRequests.delete(pending), () => pendingRequests.delete(pending));
  });
  await new Promise((resolve, reject) => { server.once('error', reject);server.listen(0, '127.0.0.1', resolve); });
  origin = `http://127.0.0.1:${server.address().port}`;
  const pageUrl = `${origin}/${pageToken}/`;
  const manifestPath = join(reportDirectory, `media-server-${runId}.json`);
  await writeFile(manifestPath, `${JSON.stringify({ schema: 'nomifun.local-media-server.v2', started_at: new Date().toISOString(),
    attempt_schema: 'nomifun.local-media-attempt.v2', capture_modes: captureModes,
    page_url: pageUrl, binding: '127.0.0.1', run_id: runId, status: 'listening', media_captures_retained: false }, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
  console.log(JSON.stringify({ status: 'listening', url: pageUrl, report: manifestPath }));
  let closing;
  const close = () => closing ??= (async () => {
    const closed = new Promise(resolve => server.close(resolve));server.closeIdleConnections();server.closeAllConnections();await closed;
    await Promise.allSettled([...pendingRequests]);
    const endPath = join(reportDirectory, `media-server-ended-${runId}.json`);
    await writeFile(endPath, `${JSON.stringify({ run_id: runId, ended_at: new Date().toISOString(), attempts_received: attemptCount,
      attempt_reports: reportFiles, scope: 'Server closure does not substitute for page track cleanup or native product acceptance' }, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
    return endPath;
  })();
  return { server, origin, pageUrl, manifestPath, close };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (process.argv.includes('--help')) console.log('node scripts/validation/global-os-media-fixture.mjs --report-dir /ABS/ISOLATED/EVIDENCE. Opens only a random loopback fixture server; it never operates UI. Open the printed nonce URL in the actual visible native Browser and click its test button. Camera/microphone permission remains native user interaction. No media, samples, device names or IDs are displayed, stored or uploaded. Every sanitized attempt gets a new 0600 report. Ctrl+C stops this server.');
  else {
    const at = process.argv.indexOf('--report-dir');assert(at >= 0 && process.argv[at + 1], '--report-dir is required');
    const fixture = await startGlobalOsMediaFixture({ reportDirectory: process.argv[at + 1] });
    const stop = () => { fixture.close().then(() => process.exit(0), () => process.exit(1)); };
    process.once('SIGINT', stop);process.once('SIGTERM', stop);
  }
}
