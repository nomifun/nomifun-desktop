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
        "/redirect-final" => html(&mut stream, 200, &simple_page("Compatibility redirects settled", "")),
        "/forbidden" => html(&mut stream, 403, &simple_page("Compatibility HTTP 403 site content", "")),
        "/reload" => html(&mut stream, 200, &simple_page("Compatibility reload ready", r#"
            const count = Number(sessionStorage.getItem('compat-reload-count') || 0);
            document.getElementById('state').textContent = 'reload-count=' + count;
            if (count === 0) {
              sessionStorage.setItem('compat-reload-count', '1');
              setTimeout(() => location.reload(), 120);
            }
        "#)),
        "/ua-report" | "/ua-fetch" => reply(&mut stream, 204, "text/plain", b"", ""),
        "/ua-image" => reply(&mut stream, 200, "image/gif", b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;", ""),
        _ => reply(&mut stream, 404, "text/plain", b"Fixture not found", ""),
    }
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
