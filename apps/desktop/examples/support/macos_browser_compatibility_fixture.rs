//! Disposable loopback HTTP fixtures; no proxy, external sites, or app data.
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::Duration,
};

#[derive(Clone, Debug)]
pub(crate) struct Receipt {
    pub path: String,
    pub user_agent: String,
    pub javascript_user_agent: Option<String>,
    pub case: Option<String>,
    pub storage_ok: Option<bool>,
    pub media_probe: Option<serde_json::Value>,
}

pub(crate) struct Fixture {
    pub address: SocketAddr,
    pub receipts: Arc<Mutex<Vec<Receipt>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    pub fn new() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "Fixture bind failed")?;
        let address = listener.local_addr().map_err(|_| "Fixture address failed")?;
        listener.set_nonblocking(true).map_err(|_| "Fixture listener setup failed")?;
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
                        requests.push(std::thread::spawn(move || serve(stream, receipts, stop)));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock =>
                        std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
            for request in requests { let _ = request.join(); }
        });
        Ok(Self { address, receipts, stop, worker: Some(worker) })
    }

    pub fn url(&self, path: &str) -> String { format!("http://{}{path}", self.address) }
    pub fn snapshot(&self) -> Vec<Receipt> { self.receipts.lock().unwrap().clone() }
    pub fn count(&self, path: &str) -> usize {
        self.receipts.lock().unwrap().iter().filter(|receipt| receipt.path == path).count()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

fn serve(mut stream: TcpStream, receipts: Arc<Mutex<Vec<Receipt>>>, stop: Arc<AtomicBool>) {
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
    let target = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("/");
    let Ok(url) = url::Url::parse(&format!("http://fixture.invalid{target}")) else { return; };
    let parameters: BTreeMap<_, _> = url.query_pairs().map(|(key, value)| (key.into_owned(), value.into_owned())).collect();
    let user_agent = request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("user-agent").then(|| value.trim().to_owned())
    }).unwrap_or_default();
    let path = url.path();
    {
        let mut receipts = receipts.lock().unwrap();
        if receipts.len() >= 1024 { return; }
        receipts.push(Receipt {
            path: path.into(), user_agent, javascript_user_agent: parameters.get("ua").cloned(),
            case: parameters.get("case").cloned(),
            storage_ok: parameters.get("storage_ok").map(|value| value == "true"),
            media_probe: path.starts_with("/media-").then(|| {
                let flag = |key: &str| parameters.get(key).and_then(|value| match value.as_str() { "true" => Some(true), "false" => Some(false), _ => None });
                let count = |key: &str| parameters.get(key).and_then(|value|value.parse::<u32>().ok()).filter(|value|*value <= 256);
                let error_name = parameters.get("error_name").map(|value| value.as_str()).filter(|value| matches!(*value, "NotAllowedError"|"NotFoundError"|"NotReadableError"|"SecurityError"|"AbortError"|"InvalidStateError"|"OverconstrainedError"|"TypeError"|"UnknownError"));
                let visibility = parameters.get("visibility_state").map(|value|value.as_str()).filter(|value|matches!(*value,"visible"|"hidden"));
                serde_json::json!({"secure_context":flag("secure_context"),"get_user_media":flag("get_user_media"),"enumerate_devices":flag("enumerate_devices"),"video_device_count":count("video_device_count"),"audio_device_count":count("audio_device_count"),"error_name":error_name,
                    "visibility_state":visibility,"document_has_focus":flag("document_has_focus"),"trusted":flag("trusted")})
            }),
        });
    }
    match path {
        "/slow-first" => {
            // No headers or document commit: Stop must not report a successful page.
            let deadline = std::time::Instant::now() + Duration::from_secs(25);
            while !stop.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        "/disconnect" => {}, // Drop the connection before any response exists.
        "/attachment" => reply(&mut stream, 200, "application/octet-stream", b"Compatibility attachment fixture bytes", "Content-Disposition: attachment; filename=\"compatibility-fixture.txt\"\r\n"),
        "/redirect-a" => redirect(&mut stream, "/redirect-b?token=redirect-secret"),
        "/redirect-b" => redirect(&mut stream, "/redirect-final?token=redirect-secret"),
        "/ua-root" => html(&mut stream, 200, &ua_page("root", "Compatibility UA root", true)),
        "/ua-frame" => html(&mut stream, 200, &ua_page("frame", "Compatibility UA frame", false)),
        "/popup" => html(&mut stream, 200, &ua_page("popup", "Compatibility popup ready", false)),
        "/identity-warm" => reply(&mut stream, 200, "text/html; charset=utf-8", identity_warm_page().as_bytes(),
            "Set-Cookie: compatibility-fixture=warm; Path=/; SameSite=Lax\r\n"),
        "/identity-check" => html(&mut stream, 200, &identity_check_page()),
        "/identity-worker.js" => reply(&mut stream, 200, "application/javascript", br#"
            postMessage(navigator.userAgent);
            fetch('/ua-report?case=worker&ua=' + encodeURIComponent(navigator.userAgent));
        "#, ""),
        "/redirect-final" => html(&mut stream, 200, &simple_page("Compatibility redirects settled", "")),
        "/forbidden" => html(&mut stream, 403, &simple_page("Compatibility HTTP 403 site content", "")),
        "/media-permission" => {
            let case = serde_json::to_string(&parameters.get("case").cloned().unwrap_or_default()).unwrap();
            let user_ready = parameters.get("user_ready").is_some_and(|value|value == "true");
            html(&mut stream, 200, &simple_page("Compatibility media permission", &format!(r#"
                const media = navigator.mediaDevices;
                const probe = () => ({{case:{case}, secure_context:isSecureContext, get_user_media:typeof media?.getUserMedia === 'function', enumerate_devices:typeof media?.enumerateDevices === 'function', visibility_state:document.visibilityState, document_has_focus:document.hasFocus()}});
                const safeName = error => ['NotAllowedError','NotFoundError','NotReadableError','SecurityError','AbortError','InvalidStateError','OverconstrainedError','TypeError'].includes(error?.name) ? error.name : 'UnknownError';
                const report = (path, extra = {{}}) => fetch(path + '?' + new URLSearchParams({{...probe(),...extra}}));
                void report('/media-probe');
                document.addEventListener('visibilitychange', () => {{ void report('/media-visibility-probe'); }});
                window.addEventListener('focus', () => {{ void report('/media-focus-probe'); }});
                window.addEventListener('blur', () => {{ void report('/media-focus-probe'); }});
                if (probe().enumerate_devices) {{
                  media.enumerateDevices().then(devices => report('/media-device-probe', {{video_device_count:devices.filter(device => device.kind === 'videoinput').length,audio_device_count:devices.filter(device => device.kind === 'audioinput').length}}), error => report('/media-device-probe', {{error_name:safeName(error)}}));
                }}
                const start = () => {{
                  if (!probe().get_user_media) {{ void report('/media-denied', {{error_name:'TypeError'}}); return; }}
                  try {{
                    media.getUserMedia({{video:true}}).then(stream => {{
                      stream.getTracks().forEach(track => track.stop());
                      void report('/media-unexpected-grant');
                    }}, error => report('/media-denied', {{error_name:safeName(error)}}));
                  }} catch (error) {{ void report('/media-denied', {{error_name:safeName(error)}}); }}
                }};
                if ({user_ready}) {{
                  const button = document.createElement('button');
                  button.type = 'button'; button.textContent = 'Start media permission: ' + {case};
                  button.style.cssText = 'font:20px system-ui;padding:18px;margin:16px';
                  document.body.append(button);
                  button.onclick = event => {{
                    if (!event.isTrusted || document.visibilityState !== 'visible' || !document.hasFocus()) {{
                      void report('/media-start-blocked', {{trusted:event.isTrusted}}); return;
                    }}
                    button.disabled = true;
                    void report('/media-user-start', {{trusted:event.isTrusted}});
                    start();
                  }};
                }} else setTimeout(start, 300);
            "#)));
        }
        "/reload" => html(&mut stream, 200, &simple_page("Compatibility reload ready", r#"
            const count = Number(sessionStorage.getItem('compat-reload-count') || 0);
            document.getElementById('state').textContent = 'reload-count=' + count;
            if (count === 0) {
              sessionStorage.setItem('compat-reload-count', '1');
              setTimeout(() => location.reload(), 120);
            }
        "#)),
        "/loop-reload" => html(&mut stream, 200, &simple_page("Compatibility continuous reload", "setInterval(() => location.reload(), 180);")),
        "/loop-replace" => html(&mut stream, 200, &simple_page("Compatibility continuous replace", "setInterval(() => location.replace('/loop-replace'), 180);")),
        "/loop-a" => html(&mut stream, 200, &simple_page("Compatibility alternating A", "setInterval(() => location.replace('/loop-b'), 180);")),
        "/loop-b" => html(&mut stream, 200, &simple_page("Compatibility alternating B", "setInterval(() => location.replace('/loop-a'), 180);")),
        "/loop-meta" => html(&mut stream, 200, "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"1;url=/loop-meta\"><title>Compatibility continuous meta refresh</title><h1>Compatibility continuous meta refresh</h1>"),
        "/resource-burst" => html(&mut stream, 200, r#"<!doctype html><meta charset="utf-8"><title>Compatibility resource burst</title>
            <h1>Compatibility resource burst</h1><iframe title="Burst child" src="/ua-frame"></iframe>
            <script>for (let i = 0; i < 30; i++) {
              fetch('/ua-fetch?burst=' + i);
              const image = new Image(); image.src = '/ua-image?burst=' + i; document.body.append(image);
            }</script>"#),
        "/ua-report" | "/ua-fetch" | "/storage-report" | "/media-probe" | "/media-device-probe" | "/media-visibility-probe" | "/media-focus-probe" | "/media-start-blocked" | "/media-user-start" | "/media-denied" | "/media-unexpected-grant" => reply(&mut stream, 204, "text/plain", b"", ""),
        "/ua-image" => reply(&mut stream, 200, "image/gif", b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;", ""),
        _ => reply(&mut stream, 404, "text/plain", b"Fixture not found", ""),
    }
}

fn identity_warm_page() -> String {
    simple_page("Compatibility profile warm", r#"
        localStorage.setItem('compatibility-profile', 'warm');
        const request = indexedDB.open('compatibility-profile', 1);
        request.onupgradeneeded = () => request.result.createObjectStore('state');
        request.onsuccess = () => {
          const db = request.result;
          const tx = db.transaction('state', 'readwrite');
          tx.objectStore('state').put('warm', 'status');
          tx.oncomplete = () => {
            db.close();
            fetch('/storage-report?case=warm&storage_ok=true');
            document.getElementById('state').textContent = 'Profile warm complete';
          };
        };
    "#)
}

fn identity_check_page() -> String {
    format!(r#"<!doctype html><meta charset="utf-8"><title>Compatibility identity comparison</title>
        <style>body{{font:18px system-ui;padding:24px}}iframe{{width:500px;height:160px}}a{{display:block;padding:16px}}</style>
        <h1>Compatibility identity comparison</h1><p id="javascript-ua"></p><p id="worker-ua"></p>
        <p id="storage">Profile checks pending</p>
        <iframe title="Compatibility UA frame" src="/ua-frame?case=frame"></iframe>
        <img alt="Fixture pixel" src="/ua-image?case=root">
        <a href="/popup" target="identity-popup">Open compatibility popup</a>
        <script>
          document.getElementById('javascript-ua').textContent = navigator.userAgent;
          fetch('/ua-report?case=root&ua=' + encodeURIComponent(navigator.userAgent));
          fetch('/ua-fetch?case=root');
          const worker = new Worker('/identity-worker.js');
          worker.onmessage = event => document.getElementById('worker-ua').textContent = event.data;
          const request = indexedDB.open('compatibility-profile', 1);
          request.onsuccess = () => {{
            const db = request.result;
            if (!db.objectStoreNames.contains('state')) {{ db.close(); return; }}
            const tx = db.transaction('state', 'readonly');
            const read = tx.objectStore('state').get('status');
            read.onsuccess = () => {{
              const ok = read.result === 'warm' && localStorage.getItem('compatibility-profile') === 'warm'
                && document.cookie.split(';').some(value => value.trim() === 'compatibility-fixture=warm');
              fetch('/storage-report?case=check&storage_ok=' + ok);
              document.getElementById('storage').textContent = ok ? 'Profile checks passed' : 'Profile checks failed';
            }};
            tx.oncomplete = () => db.close();
          }};
        </script>"#)
}

fn ua_page(case: &str, title: &str, root: bool) -> String {
    let extra = if root {
        r#"<a href="/popup?token=popup-secret" target="compatibility-popup">Open compatibility popup</a>
           <iframe title="Compatibility UA frame" src="/ua-frame?case=frame"></iframe>
           <img alt="Fixture pixel" src="/ua-image?case=root">"#
    } else { "" };
    format!(r#"<!doctype html><meta charset="utf-8"><title>{title}</title>
        <style>body{{font:18px system-ui;padding:24px}}a{{display:block;padding:16px}}iframe{{width:500px;height:160px}}</style>
        <h1>{title}</h1><p id="javascript-ua"></p>{extra}
        <script>
          document.getElementById('javascript-ua').textContent = navigator.userAgent;
          fetch('/ua-report?case={case}&ua=' + encodeURIComponent(navigator.userAgent));
          fetch('/ua-fetch?case={case}');
        </script>"#)
}

fn simple_page(title: &str, script: &str) -> String {
    format!(r#"<!doctype html><meta charset="utf-8"><title>{title}</title>
        <style>body{{font:18px system-ui;padding:24px}}</style><h1>{title}</h1><p id="state"></p><script>{script}</script>"#)
}
fn redirect(stream: &mut TcpStream, location: &str) {
    reply(stream, 302, "text/plain", b"", &format!("Location: {location}\r\n"));
}
fn html(stream: &mut TcpStream, status: u16, body: &str) { reply(stream, status, "text/html; charset=utf-8", body.as_bytes(), ""); }
fn reply(stream: &mut TcpStream, status: u16, mime: &str, body: &[u8], extra: &str) {
    let headers = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n", body.len());
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(body);
}
