//! Disposable native-navigation witnesses. Request counts do not infer frame scope.
//! Each route has an explicit role, and receipts retain no query, credentials or body.
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

const RECEIPT_LIMIT: usize = 2048;

#[derive(Clone, Debug)]
pub(crate) struct Receipt {
    pub path: String,
    pub destination: String,
    pub method: String,
    pub user_agent: String,
    /// A controlled integer witness for the POST resume test, never a body.
    pub post_attempt: Option<u64>,
}

pub(crate) struct Fixture {
    address: SocketAddr,
    receipts: Arc<Mutex<Vec<Receipt>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    pub fn new() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "Navigation fixture bind failed")?;
        let address = listener.local_addr().map_err(|_| "Navigation fixture address failed")?;
        listener.set_nonblocking(true).map_err(|_| "Navigation fixture listener setup failed")?;
        let stop = Arc::new(AtomicBool::new(false));
        let receipts = Arc::new(Mutex::new(Vec::new()));
        let worker_stop = stop.clone();
        let worker_receipts = receipts.clone();
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let stop = worker_stop.clone();
                        let receipts = worker_receipts.clone();
                        requests.push(std::thread::spawn(move || serve(stream, address, receipts, stop)));
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
            for request in requests { let _ = request.join(); }
        });
        Ok(Self { address, receipts, stop, worker: Some(worker) })
    }

    pub fn url(&self, path: &str) -> String { format!("http://{}{path}", self.address) }
    pub fn count(&self, path: &str) -> usize { self.snapshot().iter().filter(|receipt| receipt.path == path).count() }
    pub fn snapshot(&self) -> Vec<Receipt> { self.receipts.lock().unwrap().clone() }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn serve(mut stream: TcpStream, address: SocketAddr, receipts: Arc<Mutex<Vec<Receipt>>>, stop: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut request = Vec::new();
    let mut buffer = [0; 2048];
    while request.len() <= 16 * 1024 && !request.windows(4).any(|value| value == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(size) => request.extend_from_slice(&buffer[..size]),
        }
    }
    if request.len() > 16 * 1024 { return; }
    let request = String::from_utf8_lossy(&request);
    let mut first = request.lines().next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_owned();
    let target = first.next().unwrap_or("/");
    let Ok(url) = url::Url::parse(&format!("http://fixture.invalid{target}")) else { return; };
    let header = |name: &str| request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim().to_owned())
    }).unwrap_or_default();
    // Consume a bounded synthetic POST body without parsing or retaining it,
    // so close() cannot reset a successful fixture response with unread data.
    let body_length=header("content-length").parse::<usize>().unwrap_or(0);
    if body_length>16*1024 {return;}
    let header_end=request.find("\r\n\r\n").map_or(request.len(),|offset|offset+4);
    let mut unread=body_length.saturating_sub(request.len().saturating_sub(header_end));
    while unread>0 {
        let take=unread.min(buffer.len());
        match stream.read(&mut buffer[..take]) {Ok(0)|Err(_)=>return,Ok(size)=>unread=unread.saturating_sub(size)}
    }
    let path = url.path();
    {
        let mut values = receipts.lock().unwrap();
        if values.len() >= RECEIPT_LIMIT { return; }
        let post_attempt=if path=="/nav/post-receive" {url.query_pairs().find(|(key,_)|key=="attempt")
            .and_then(|(_,value)|value.parse::<u64>().ok()).filter(|value|*value<=10000)}else{None};
        values.push(Receipt { path: path.into(), destination: header("sec-fetch-dest"),
            method, user_agent: header("user-agent"),post_attempt });
    }
    match path {
        "/nav/initial" | "/nav/final" | "/nav/replacement" | "/nav/race-final" | "/nav/bounded" | "/nav/recovery" =>
            html(&mut stream, 200, &page(path, "")),
        "/nav/history-base" => html(&mut stream,200,&format!(r#"{}
            <button id=history-one>Push history step one</button><button id=history-two>Push history step two</button>
            <script>window.fixtureHistoryClicks=[];
                for(const [id,path] of [['history-one','/nav/history-spa-1'],['history-two','/nav/history-spa-2']])
                    document.getElementById(id).addEventListener('click',event=>{{
                        fixtureHistoryClicks.push({{id,trusted:event.isTrusted}});history.pushState({{id}},'',path);
                    }});
            </script>"#,page(path,""))),
        "/nav/history-other" => html(&mut stream,200,&format!(r#"{}
            <button id=history-leave onclick="window.fixtureHistoryLeaveTrusted=event.isTrusted;window.onbeforeunload=event=>{{event.preventDefault();event.returnValue=''}}">Arm history leave confirmation</button>"#,page(path,""))),
        "/nav/history-spa-1" | "/nav/history-spa-2" => html(&mut stream,200,&page(path,"")),
        "/nav/redirect-a" => redirect(&mut stream, "/nav/redirect-b?fixture_secret=redirect"),
        "/nav/redirect-b" => redirect(&mut stream, "/nav/final?fixture_secret=redirect#private_fragment"),
        "/nav/script-reload" => html(&mut stream, 200, &page(path, r#"
            const count = Number(sessionStorage.getItem('nav-fixture-once') || 0);
            document.getElementById('state').textContent = 'reload-count=' + count;
            if (!count) { sessionStorage.setItem('nav-fixture-once', '1'); setTimeout(() => location.reload(), 100); }
        "#)),
        "/nav/script-replace" => html(&mut stream, 200, &page(path, "setTimeout(() => location.replace('/nav/replacement'), 100);")),
        "/nav/meta" => html(&mut stream, 200, &format!("<meta http-equiv=refresh content=\"0.2;url=/nav/replacement\">{}", page(path, ""))),
        "/nav/loop" => html(&mut stream, 200, &page(path, "setInterval(() => location.reload(), 180);")),
        "/nav/aba-a" => html(&mut stream, 200, &page(path, "setInterval(() => location.replace('/nav/aba-b'), 180);")),
        "/nav/aba-b" => html(&mut stream, 200, &page(path, "setInterval(() => location.replace('/nav/aba-a'), 180);")),
        "/nav/post-source" => html(&mut stream,200,&page(path,r#"
            window.fixturePostOrdinal=0;
            const form=document.createElement('form');form.method='POST';
            const input=document.createElement('input');input.name='fixture_only';input.value='synthetic-local-body';
            form.append(input);document.body.append(form);
            setTimeout(()=>setInterval(()=>{
                const ordinal=++fixturePostOrdinal;form.action='/nav/post-receive?attempt='+ordinal;form.requestSubmit();
            },80),800);
        "#)),
        "/nav/post-receive" => html(&mut stream,200,&page(path,"")),
        "/nav/slow" | "/nav/slow-pause" | "/nav/slow-late" | "/nav/provisional-guard" | "/nav/recovery-slow"
        | "/nav/workspace-stop" | "/nav/workspace-pause" | "/nav/late-hidden" => {
            let delay = if matches!(path,"/nav/slow-late"|"/nav/late-hidden"|"/nav/recovery-slow") { 1200 } else { 8000 };
            let deadline = Instant::now() + Duration::from_millis(delay);
            while !stop.load(Ordering::Acquire) && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(10)); }
            if !stop.load(Ordering::Acquire) { html(&mut stream, 200, &format!("{}<input id=recovery-input oninput=\"window.fixtureRecoveryInputTrusted=event.isTrusted\">",page(path, ""))); }
        },
        "/nav/disconnect" => {},
        "/nav/forbidden" => html(&mut stream, 403, &page("HTTP 403 fixture document", "")),
        "/nav/download" => reply(&mut stream, 200, "application/octet-stream", b"Navigation fixture attachment", "Content-Disposition: attachment; filename=\"navigation-fixture.txt\"\r\n"),
        "/nav/resources" | "/nav/resources-strong-worker" | "/nav/resources-worker-executed" | "/nav/resources-worker-plain" => html(&mut stream, 200, &format!(r#"{}
            <script>
              window.fixtureFrames = [];
              addEventListener('message', event => {{ if (event.data?.fixtureKind === 'frame') fixtureFrames.push(event.data); }});
              for (const [id, src] of [['same-frame','/nav/frame'],['cross-frame','http://localhost:{}/nav/frame']]) {{
                const frame=document.createElement('iframe'); frame.id=id; frame.src=src; document.body.append(frame);
              }}
              window.fixtureWorker = null; window.fixtureNestedWorker=null; window.fixtureWorkerError=null;
              const worker = new Worker('{}'); {}
              worker.onmessage = event => {{ if(event.data?.fixtureKind==='nested_worker') fixtureNestedWorker=event.data.worker;
                else fixtureWorker = event.data; }};
              worker.onerror = event => fixtureWorkerError = {{kind:'worker_error'}};
              for (let n = 0; n < 12; n++) {{ fetch('/nav/fetch?n=' + n); const image = new Image(); image.src='/nav/pixel?n=' + n; document.body.append(image); }}
              window.fixturePoll = setInterval(() => fetch('/nav/fetch?poll=1'), 90);
            </script>"#, page(path, ""), address.port(), if path=="/nav/resources-worker-plain" {"/nav/worker-plain.js"}
                else if path=="/nav/resources-worker-executed" {"/nav/worker-diagnostic.js"} else {"/nav/worker.js"},
                if path=="/nav/resources-strong-worker" {"window.fixtureWorkerHandle=worker;"} else {""})),
        "/nav/frame" => html(&mut stream, 200, &page(path, r#"
            parent.postMessage({fixtureKind:'frame', ua:navigator.userAgent,
                isTauri: 'isTauri' in window, internals:'__TAURI_INTERNALS__' in window,
                ipc: 'ipc' in window, dialog: [typeof alert, typeof confirm, typeof prompt]}, '*');
        "#)),
        "/nav/worker.js" => reply(&mut stream, 200, "application/javascript", b"postMessage({ua:navigator.userAgent,isTauri:'isTauri' in self,internals:'__TAURI_INTERNALS__' in self,ipc:'ipc' in self});const nested=new Worker('/nav/nested-worker.js');nested.onmessage=event=>postMessage({fixtureKind:'nested_worker',worker:event.data});", ""),
        "/nav/nested-worker.js" => reply(&mut stream,200,"application/javascript",b"fetch('/nav/nested-worker-executed').catch(()=>{});postMessage({ua:navigator.userAgent,isTauri:'isTauri' in self,internals:'__TAURI_INTERNALS__' in self,ipc:'ipc' in self});",""),
        "/nav/nested-worker-executed" => reply(&mut stream,204,"text/plain",b"",""),
        "/nav/worker-diagnostic.js" => reply(&mut stream,200,"application/javascript",
            b"fetch('/nav/worker-executed').catch(()=>{});postMessage({ua:navigator.userAgent,isTauri:'isTauri' in self,internals:'__TAURI_INTERNALS__' in self,ipc:'ipc' in self});",""),
        "/nav/worker-executed" => reply(&mut stream,204,"text/plain",b"",""),
        "/nav/worker-plain.js" => reply(&mut stream,200,"application/javascript",
            b"fetch('/nav/worker-plain-executed').catch(()=>{});postMessage({ua:navigator.userAgent,isTauri:'isTauri' in self,internals:'__TAURI_INTERNALS__' in self,ipc:'ipc' in self});",""),
        "/nav/worker-plain-executed" => reply(&mut stream,204,"text/plain",b"",""),
        "/nav/fetch" => reply(&mut stream, 204, "text/plain", b"", ""),
        "/nav/pixel" => reply(&mut stream, 200, "image/gif", b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;", ""),
        "/nav/popup-source" => html(&mut stream, 200, &format!(r#"{}
            <a id=popup href=/nav/popup target=native-navigation-popup>Open navigation popup</a>
            <a id=blank-popup href=/nav/popup-blank target=_blank>Open new blank-target navigation popup</a>
            <button id=script-popup>Open named popup from trusted handler</button>
            <button id=writable-popup>Open writable production blank popup</button>
            <script>
                window.fixturePopupClicks=[];
                for(const anchor of document.querySelectorAll('a')) anchor.addEventListener('click',event=>{{
                    fixturePopupClicks.push({{target:anchor.id,trusted:event.isTrusted}});
                }});
                document.getElementById('script-popup').addEventListener('click',event=>{{
                    fixturePopupClicks.push({{target:'script-popup',trusted:event.isTrusted}});
                    window.open('/nav/popup','native-navigation-popup');
                }});
                document.getElementById('writable-popup').addEventListener('click',event=>{{
                    fixturePopupClicks.push({{target:'writable-popup',trusted:event.isTrusted}});
                    const child=window.open('about:blank','native-navigation-written');
                    child.document.write('<!doctype html><title>Written production popup</title><h1 id=written-native>Written by opener</h1><input id=written-input oninput="window.fixtureBlankInputTrusted=event.isTrusted">');
                    child.document.close();
                }});
            </script>"#, page(path, ""))),
        "/nav/popup" => html(&mut stream, 200, &page(path, "")),
        "/nav/popup-blank" => html(&mut stream, 200, &page(path, "")),
        _ => reply(&mut stream, 404, "text/plain", b"Navigation fixture route not found", ""),
    }
}

fn page(title: &str, script: &str) -> String {
    format!(r#"<!doctype html><meta charset=utf-8><title>{title}</title>
        <style>body{{font:18px system-ui;padding:24px}}a{{display:block;padding:16px}}</style>
        <h1>{title}</h1><p id=state>fixture-ready</p><script>window.fixtureDocumentNonce=crypto.randomUUID();{script}</script>"#)
}
fn html(stream: &mut TcpStream, status: u16, body: &str) { reply(stream, status, "text/html; charset=utf-8", body.as_bytes(), ""); }
fn redirect(stream: &mut TcpStream, location: &str) { reply(stream, 302, "text/plain", b"", &format!("Location: {location}\r\n")); }
fn reply(stream: &mut TcpStream, status: u16, mime: &str, body: &[u8], extra: &str) {
    let headers = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n", body.len());
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(fixture: &Fixture, target: &str) -> String {
        let mut stream = TcpStream::connect(fixture.address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: fixture\r\nSec-Fetch-Dest: document\r\nUser-Agent: fixture-agent\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn server_counts_requests_and_strips_secrets_from_receipts() {
        let fixture = Fixture::new().unwrap();
        let redirect = request(&fixture, "/nav/redirect-a?fixture_secret=sensitive");
        assert!(redirect.starts_with("HTTP/1.1 302"));
        assert!(redirect.contains("Location: /nav/redirect-b"));
        assert!(request(&fixture, "/nav/initial").contains("fixture-ready"));
        assert_eq!(fixture.count("/nav/redirect-a"), 1);
        assert_eq!(fixture.count("/nav/initial"), 1);
        let receipts = fixture.snapshot();
        assert_eq!(receipts[0].destination, "document");
        assert_eq!(receipts[0].method, "GET");
        assert_eq!(receipts[0].user_agent, "fixture-agent");
        assert!(!format!("{receipts:?}").contains("sensitive"));
    }

    #[test]
    fn fixture_declares_reload_replace_meta_and_resources_independently() {
        let fixture = Fixture::new().unwrap();
        assert!(request(&fixture, "/nav/loop").contains("setInterval(() => location.reload()"));
        assert!(request(&fixture, "/nav/script-replace").contains("location.replace"));
        assert!(request(&fixture, "/nav/meta").contains("http-equiv=refresh"));
        let resources = request(&fixture, "/nav/resources");
        assert!(resources.contains("new Worker"));
        assert!(resources.contains("iframe"));
        assert!(resources.contains("setInterval(() => fetch"));
        assert_eq!(fixture.count("/nav/fetch"), 0);
    }

    #[test]
    fn post_witness_keeps_only_controlled_ordinal_not_body() {
        let fixture=Fixture::new().unwrap();
        let mut stream=TcpStream::connect(fixture.address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let body="synthetic_body_must_not_enter_receipts";
        write!(stream,"POST /nav/post-receive?attempt=7 HTTP/1.1\r\nHost: fixture\r\nSec-Fetch-Dest: document\r\nContent-Length: {}\r\n\r\n{}",body.len(),body).unwrap();
        let mut response=String::new();stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        let receipts=fixture.snapshot();
        assert_eq!(receipts.len(),1);assert_eq!(receipts[0].method,"POST");assert_eq!(receipts[0].post_attempt,Some(7));
        assert!(!format!("{receipts:?}").contains(body));
    }
}
