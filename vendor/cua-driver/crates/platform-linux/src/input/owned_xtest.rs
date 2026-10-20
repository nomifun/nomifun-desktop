//! An opt-in, process-owned XTEST pair with cleanup outside the worker group.
//!
//! Embedders must prepare this before starting any other threads. Normal Cua
//! clients that do not opt in keep their existing input routing. No key/button
//! ledger is maintained: the X server owns the state of this exact device pair.

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex, OnceLock};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xinput::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

static INPUT_OWNER: OnceLock<InputContext> = OnceLock::new();
type SharedChannel = Arc<Mutex<Option<Channel>>>;

struct Channel {
    writer: File,
    reader: BufReader<File>,
}

struct InputContext {
    pair: OnceLock<DevicePair>,
    core_pointer: u16,
    name: String,
    channel: SharedChannel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DevicePair {
    pointer: u16,
    keyboard: u16,
    xtest_pointer: u16,
    xtest_keyboard: u16,
    core_pointer: u16,
    name: String,
}

#[derive(Serialize, Deserialize)]
enum Startup {
    Isolated { pid: i32 },
    Prepared { core_pointer: u16, name: String },
    Ready(DevicePair),
    Ok,
    Borrowed(u64),
    Released { restored: bool },
    Failed(String),
}

#[derive(Serialize, Deserialize)]
enum NativeCommand {
    Refresh,
    Borrow { keycode: u8, keysyms: Vec<u32> },
    Restore { token: u64 },
}

/// Exact temporary cleanup owner. Keep this on the embedding process's stack
/// until its SDK runtime has shut down, then call `finish` and check its result.
#[must_use = "the original process must retain its XTEST cleanup owner"]
pub struct ProcessXtestOwner {
    guardian: libc::pid_t,
    channel: SharedChannel,
    finished: bool,
}

impl ProcessXtestOwner {
    /// Close the lifetime pipe and wait for actual native cleanup. A signal,
    /// nonzero guardian exit or wait failure is an error, never cleanup proof.
    pub fn finish(mut self) -> Result<()> {
        self.finish_inner()
    }

    fn finish_inner(&mut self) -> Result<()> {
        self.channel
            .lock()
            .map_err(|_| anyhow!("XTEST guardian channel poisoned"))?
            .take();
        if self.finished {
            return Ok(());
        }
        let mut status = 0;
        loop {
            let result = unsafe { libc::waitpid(self.guardian, &mut status, 0) };
            if result == self.guardian {
                self.finished = true;
                ensure!(
                    libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
                    "owned XTEST guardian did not confirm cleanup (status {status})"
                );
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error).context("wait for owned XTEST guardian");
            }
        }
    }
}

impl Drop for ProcessXtestOwner {
    fn drop(&mut self) {
        if let Err(error) = self.finish_inner() {
            eprintln!("owned XTEST cleanup failed: {error:#}");
        }
    }
}

/// Prepare independent cleanup for foreground X11 input. No DISPLAY means no
/// X server resources or child process are created (metadata-only clients).
///
/// # Safety
/// Call once, on the process's initial thread, before creating any SDK, Tokio,
/// Xlib or other worker threads. The child continues Rust/Xlib after `fork`,
/// which is only valid here because the calling process is still single-threaded.
pub unsafe fn prepare_process_xtest_owner() -> Result<Option<ProcessXtestOwner>> {
    if std::env::var_os("DISPLAY").is_none() {
        return Ok(None);
    }
    ensure!(INPUT_OWNER.get().is_none(), "owned XTEST already prepared");
    let owner = super::mpx_owner::Owner::current().context("identify XTEST worker owner")?;
    // Reuse the pinned driver's incarnation-aware device identity. One prepare
    // is permitted per process, so the exact owner + nonce is unique.
    let mut random = [0; 8];
    getrandom::fill(&mut random).map_err(|error| anyhow!("generate XTEST owner nonce: {error}"))?;
    let name = owner.master_name(u64::from_ne_bytes(random));
    let expected_name = name.clone();
    let (life_read, life_write) = pipe()?;
    let (ready_read, ready_write) = pipe()?;
    let pid = unsafe { libc::fork() };
    if pid == -1 {
        return Err(std::io::Error::last_os_error()).context("fork owned XTEST guardian");
    }
    if pid == 0 {
        drop(life_write);
        drop(ready_read);
        let code = match guardian(life_read, ready_write, name) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("owned XTEST guardian: {error:#}");
                1
            }
        };
        unsafe { libc::_exit(code) };
    }
    drop(life_read);
    drop(ready_write);
    // Until READY, retain the writer in this local cleanup owner too.
    let channel = Arc::new(Mutex::new(Some(Channel {
        writer: File::from(life_write),
        reader: BufReader::new(File::from(ready_read)),
    })));
    let guard = ProcessXtestOwner {
        guardian: pid,
        channel: channel.clone(),
        finished: false,
    };
    let (core_pointer, name) = {
        let mut locked = channel
            .lock()
            .map_err(|_| anyhow!("XTEST guardian channel poisoned"))?;
        let reader = &mut locked
            .as_mut()
            .ok_or_else(|| anyhow!("XTEST startup channel closed"))?
            .reader;
        let isolated = startup(reader)?;
        ensure!(
            matches!(isolated, Startup::Isolated { pid: child } if child == pid),
            "XTEST guardian did not establish an independent session"
        );
        match startup(reader)? {
            Startup::Prepared { core_pointer, name } => (core_pointer, name),
            Startup::Failed(error) => bail!("owned XTEST startup failed: {error}"),
            _ => bail!("invalid owned XTEST startup response"),
        }
    };
    ensure!(
        name.starts_with(&format!("{expected_name} guarded "))
            && name.split_once(" guarded ").is_some_and(|(_, guardian)| {
                super::mpx_owner::Owner::from_pointer_name(&format!("{guardian} pointer")).is_some()
            }),
        "owned XTEST identity changed during startup"
    );
    INPUT_OWNER
        .set(InputContext {
            pair: OnceLock::new(),
            core_pointer,
            name,
            channel,
        })
        .map_err(|_| anyhow!("owned XTEST already prepared"))?;
    Ok(Some(guard))
}

fn startup(reader: &mut impl BufRead) -> Result<Startup> {
    let mut line = String::new();
    let count = reader.take(4096).read_line(&mut line)?;
    ensure!(
        count > 0 && count < 4096,
        "XTEST guardian startup EOF or oversized response"
    );
    serde_json::from_str(&line).context("decode XTEST guardian startup")
}

fn send_startup(writer: &mut File, value: &Startup) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    ensure!(
        unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == 0,
        "create XTEST guardian pipe: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn guardian(life: OwnedFd, ready: OwnedFd, worker_name: String) -> Result<()> {
    // No server resource exists until the child is outside the original group.
    ensure!(
        unsafe { libc::setsid() } != -1,
        "isolate XTEST guardian session: {}",
        std::io::Error::last_os_error()
    );
    unsafe {
        ensure!(
            libc::prctl(libc::PR_SET_PDEATHSIG, 0) == 0,
            "clear guardian parent-death signal: {}",
            std::io::Error::last_os_error()
        );
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    close_unrelated_fds(life.as_raw_fd(), ready.as_raw_fd())?;
    let mut ready = File::from(ready);
    send_startup(
        &mut ready,
        &Startup::Isolated {
            pid: unsafe { libc::getpid() },
        },
    )?;
    // A generic worker-only reaper must not delete devices while their live
    // guardian is draining clients. Both complete owners are atomic in the
    // XIAddMaster name; the old v1 parser deliberately cannot recognize it.
    let guardian_owner = super::mpx_owner::Owner::current().context("identify XTEST guardian")?;
    let name = format!("{worker_name} guarded {}", guardian_owner.master_name(0));
    let display = match super::open_display() {
        Ok(display) => display,
        Err(error) => {
            let _ = send_startup(&mut ready, &Startup::Failed(error.to_string()));
            return Err(error);
        }
    };
    // Child-local handler only, installed after open_display initializes Xlib.
    unsafe { x11::xlib::XSetErrorHandler(Some(super::ignore_x_error)) };
    // Fix this client's core selection before adding another master. XIGet-
    // ClientPointer alone is lazy and may report no selection on a new client.
    unsafe {
        let (mut focus, mut revert) = (0, 0);
        x11::xlib::XGetInputFocus(display, &mut focus, &mut revert);
    }
    let mut core_pointer = 0;
    ensure!(
        unsafe { x11::xinput2::XIGetClientPointer(display, 0, &mut core_pointer) } != 0,
        "identify original core input pointer"
    );
    let core_pointer: u16 = core_pointer
        .try_into()
        .context("core pointer id out of range")?;
    let mut pair = None;
    let mut mirrors = Vec::new();
    let mut next_token = 1;
    let mut clients = NativeClients::default();
    let result = (|| -> Result<()> {
        send_startup(
            &mut ready,
            &Startup::Prepared {
                core_pointer,
                name: name.clone(),
            },
        )?;
        let mut life = BufReader::new(File::from(life));
        loop {
            let mut line = String::new();
            let count = (&mut life).take(16384).read_line(&mut line)?;
            if count == 0 {
                break;
            }
            ensure!(count < 16384, "oversized native XTEST resource command");
            let command: NativeCommand = serde_json::from_str(&line)?;
            let response = match command {
                NativeCommand::Refresh => (|| -> Result<Startup> {
                    if pair.is_none() {
                        pair = lookup_owned_pair(display, &name, core_pointer)?;
                    }
                    if pair.is_none() {
                        pair = Some(create_pair(display, &name)?);
                    }
                    let pair = pair
                        .as_ref()
                        .ok_or_else(|| anyhow!("owned XTEST creation missing pair"))?;
                    collect_clients(pair, &mut clients)?;
                    refresh_mapping(display, pair, &mirrors)?;
                    Ok(Startup::Ready(pair.clone()))
                })(),
                NativeCommand::Borrow { keycode, keysyms } => borrow_core_row(
                    display,
                    pair.as_ref(),
                    keycode,
                    &keysyms,
                    &mut mirrors,
                    &mut next_token,
                )
                .map(Startup::Borrowed),
                NativeCommand::Restore { token } => restore_core_row(display, token, &mut mirrors)
                    .map(|restored| Startup::Released { restored }),
            }
            .unwrap_or_else(|error| Startup::Failed(error.to_string()));
            send_startup(&mut ready, &response)?;
        }
        Ok(())
    })();
    // Keep the exact native resources and their ownership data until all
    // retirement phases finish. An observer connection or readback failure is
    // not proof that the original X server, client or borrowed row disappeared.
    let mut parking = None;
    let mut pending_phase = None;
    let mut backoff = std::time::Duration::from_millis(100);
    loop {
        let mut phase = "release and park owned input";
        let cleanup = (|| -> Result<()> {
            let mut parked = Ok(());
            if pair.is_none() {
                // Creation can finish in the server before its reply is read.
                // Recover the same named resource; never delete it as though
                // no input pair had existed or create a duplicate on retry.
                match lookup_owned_pair(display, &name, core_pointer) {
                    Ok(found) => pair = found,
                    Err(error) => parked = Err(error),
                }
            }
            if parking.is_none() {
                if let Some(pair) = pair.as_ref() {
                    match quiesce_clients(display, pair, &mut clients) {
                        Ok(window) => parking = Some(window),
                        Err(error) => parked = Err(error),
                    }
                }
            }
            // A stopped client must not keep a borrowed core row installed.
            // Completed text keeps its upstream TextClient ACK/restore order;
            // this is retirement after the embedding process stopped input.
            phase = "restore borrowed core keymap";
            let restored = restore_all_core_rows(display, &mut mirrors);
            // Parking can be pending on an application's active device grab or
            // a temporary observer failure. Its global core rows still need
            // an independent restoration attempt on every retirement pass.
            if let Err(error) = parked {
                phase = "release and park owned input";
                return Err(error);
            }
            restored?;
            phase = "drain native clients";
            if !clients.clients.is_empty() {
                wait_for_clients(&mut clients)?;
            }
            phase = "remove exact owned pair";
            remove_exact_pair(display, &name, pair.as_ref())
        })();
        match cleanup {
            Ok(()) => break,
            Err(error) => {
                if pending_phase != Some(phase) {
                    eprintln!("owned XTEST cleanup pending ({phase}): {error:#}");
                    pending_phase = Some(phase);
                }
                // Retry only retirement of these same native resources. No
                // desktop action, replacement worker or task lifetime is added.
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(std::time::Duration::from_secs(1));
            }
        }
    }
    if pending_phase.is_some() {
        eprintln!("owned XTEST cleanup completed after pending native retirement");
    }
    if let Some(parking) = parking {
        unsafe { x11::xlib::XDestroyWindow(display, parking) };
    }
    unsafe { x11::xlib::XCloseDisplay(display) };
    result
}

fn collect_clients(pair: &DevicePair, clients: &mut NativeClients) -> Result<()> {
    let (conn, screen) = RustConnection::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let pointer = conn.xinput_xi_query_pointer(root, pair.pointer)?.reply()?;
    let focus = conn.xinput_xi_get_focus(pair.keyboard)?.reply()?.focus;
    // Refresh can create the pair before the input caller has bound its
    // connection or copied focus. In that window the new devices have no
    // recipient yet, but the current foreground client still consumes the
    // hierarchy change and may schedule cursor requests for them. Include the
    // original core pair's actual clients without moving or binding it.
    let core = conn
        .xinput_xi_query_device(pair.core_pointer)?
        .reply()?
        .infos
        .into_iter()
        .find(|info| info.deviceid == pair.core_pointer)
        .ok_or_else(|| anyhow!("original core pointer disappeared"))?;
    let core_pointer = conn
        .xinput_xi_query_pointer(root, pair.core_pointer)?
        .reply()?;
    let core_focus = conn.xinput_xi_get_focus(core.attachment)?.reply()?.focus;
    for window in [pointer.child, focus, core_pointer.child, core_focus] {
        if window > 1 && window != root {
            clients.record(&conn, window)?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct NativeClients {
    // XRes resource ranges describe live X connections, not application or
    // execution state. Keep them only while this exact XI resource is retiring.
    clients: BTreeMap<(u32, u32), ClientContact>,
    xres: OnceLock<Option<(u16, u16)>>,
}

struct ClientContact {
    windows: BTreeSet<u32>,
    requires_ping: bool,
    xres: bool,
    peer_pid: Option<u32>,
}

impl NativeClients {
    fn xres_version(&self, conn: &RustConnection) -> Result<Option<(u16, u16)>> {
        use x11rb::protocol::res::ConnectionExt as _;
        if let Some(version) = self.xres.get() {
            return Ok(*version);
        }
        let version = if conn.extension_information("X-Resource")?.is_some() {
            let reply = conn.res_query_version(1, 2)?.reply()?;
            Some((reply.server_major, reply.server_minor))
        } else {
            None
        };
        let _ = self.xres.set(version);
        Ok(version)
    }

    fn record(&mut self, conn: &RustConnection, window: u32) -> Result<()> {
        use x11rb::protocol::res::ConnectionExt as _;
        let version = self.xres_version(conn)?;
        let (base, mask) = if version.is_some() {
            let Some(client) = conn
                .res_query_clients()?
                .reply()?
                .clients
                .into_iter()
                .find(|client| window & !client.resource_mask == client.resource_base)
            else {
                return Ok(()); // The connection is already gone, not BadWindow.
            };
            (client.resource_base, client.resource_mask)
        } else {
            (
                window & !conn.setup().resource_id_mask,
                conn.setup().resource_id_mask,
            )
        };
        if base == 0 {
            return Ok(());
        }
        let ping = strict_ping_window(conn, window, base, mask)?;
        let peer_pid = if version.is_some_and(|v| v >= (1, 2)) {
            native_client_pid(conn, base)?
        } else {
            None
        };
        if self.clients.get(&(base, mask)).is_some_and(|previous| {
            previous
                .peer_pid
                .zip(peer_pid)
                .is_some_and(|(old, now)| old != now)
        }) {
            // This new input is about to use a newly connected recipient that
            // reused a departed client's range. Track its own drain obligation.
            self.clients.remove(&(base, mask));
        }
        let client = self.clients.entry((base, mask)).or_insert(ClientContact {
            windows: BTreeSet::new(),
            requires_ping: false,
            xres: version.is_some(),
            peer_pid,
        });
        client.windows.insert(window);
        // A missing old window does not establish that a live X connection
        // lacks PING support. Preserve responsibility until another window or
        // XRes proves what happened to that connection.
        client.requires_ping |= ping.is_some() || !window_exists(conn, window)?;
        if let Some(window) = ping {
            client.windows.insert(window);
        }
        Ok(())
    }
}

fn native_client_pid(conn: &RustConnection, base: u32) -> Result<Option<u32>> {
    use crate::x11_client_pid::{XresClientValue, select_owner_pid};
    use x11rb::protocol::res::{ClientIdMask, ClientIdSpec, ConnectionExt as _};
    let own_base = conn.setup().resource_id_base;
    let specs = [own_base, base].map(|client| ClientIdSpec {
        client,
        mask: ClientIdMask::LOCAL_CLIENT_PID,
    });
    let reply = conn.res_query_client_ids(&specs)?.reply()?;
    let values: Vec<_> = reply
        .ids
        .iter()
        .map(|id| XresClientValue {
            client: id.spec.client,
            mask: u32::from(id.spec.mask),
            value: &id.value,
        })
        .collect();
    Ok(select_owner_pid(
        &values,
        own_base,
        std::process::id(),
        base,
    ))
}

fn window_exists(conn: &RustConnection, window: u32) -> Result<bool> {
    match conn.get_window_attributes(window)?.reply() {
        Ok(_) => Ok(true),
        Err(x11rb::errors::ReplyError::X11Error(error))
            if error.error_kind == x11rb::protocol::ErrorKind::Window =>
        {
            Ok(false)
        }
        Err(error) => Err(error).context("inspect native cleanup window"),
    }
}

fn strict_ping_window(
    conn: &RustConnection,
    mut window: u32,
    base: u32,
    mask: u32,
) -> Result<Option<u32>> {
    use x11rb::protocol::xproto::AtomEnum;
    let protocols = conn.intern_atom(false, b"WM_PROTOCOLS")?.reply()?.atom;
    let ping = conn.intern_atom(false, b"_NET_WM_PING")?.reply()?.atom;
    while window > 1 && window & !mask == base {
        if !window_exists(conn, window)? {
            return Ok(None);
        }
        let property = match conn
            .get_property(false, window, protocols, AtomEnum::ATOM, 0, 64)?
            .reply()
        {
            Ok(reply) => reply,
            Err(x11rb::errors::ReplyError::X11Error(error))
                if error.error_kind == x11rb::protocol::ErrorKind::Window =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error).context("inspect native cleanup PING support"),
        };
        if property
            .value32()
            .is_some_and(|mut atoms| atoms.any(|atom| atom == ping))
        {
            return Ok(Some(window));
        }
        window = match conn.query_tree(window)?.reply() {
            Ok(reply) => reply.parent,
            Err(x11rb::errors::ReplyError::X11Error(error))
                if error.error_kind == x11rb::protocol::ErrorKind::Window =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error).context("resolve native cleanup client window"),
        };
    }
    Ok(None)
}

fn same_client_ping_windows(
    conn: &RustConnection,
    base: u32,
    mask: u32,
    known: &BTreeSet<u32>,
) -> Result<BTreeSet<u32>> {
    let mut found = BTreeSet::new();
    for &window in known {
        if let Some(window) = strict_ping_window(conn, window, base, mask)? {
            found.insert(window);
        }
    }
    if !found.is_empty() {
        return Ok(found);
    }
    // A GTK toplevel may be destroyed while its connection and other windows
    // stay alive. Find a real protocol window in that same native resource
    // range; never treat one BadWindow as proof of application exit.
    let mut remaining: Vec<_> = conn.setup().roots.iter().map(|root| root.root).collect();
    while let Some(window) = remaining.pop() {
        if window & !mask == base {
            if let Some(window) = strict_ping_window(conn, window, base, mask)? {
                found.insert(window);
            }
        }
        match conn.query_tree(window)?.reply() {
            Ok(tree) => remaining.extend(tree.children),
            Err(x11rb::errors::ReplyError::X11Error(error))
                if error.error_kind == x11rb::protocol::ErrorKind::Window => {}
            Err(error) => return Err(error).context("find surviving native client PING window"),
        }
    }
    Ok(found)
}

fn wait_for_clients(clients: &mut NativeClients) -> Result<()> {
    use x11rb::protocol::res::ConnectionExt as _;
    let (conn, _) = RustConnection::connect(None)?;
    let (mut confirmed, mut unsupported, mut gone) = (0, 0, 0);
    let mut pending_announced = false;
    let mut backoff = std::time::Duration::from_millis(100);
    while !clients.clients.is_empty() {
        let current = if clients.xres_version(&conn)?.is_some() {
            Some(conn.res_query_clients()?.reply()?.clients)
        } else {
            None
        };
        let mut completed = Vec::new();
        for (&(base, mask), client) in &mut clients.clients {
            let disappeared = client.xres
                && current.as_ref().is_some_and(|list| {
                    !list
                        .iter()
                        .any(|item| item.resource_base == base && item.resource_mask == mask)
                });
            let replaced = if let Some(original) = client.peer_pid.filter(|_| !disappeared) {
                // The XRes peer PID is immutable for a live connection. A
                // different server-reported peer proves that this slot now
                // names another connection. Process exit or /proc start-time
                // change alone proves nothing: a forked child may hold its FD.
                native_client_pid(&conn, base)?.is_some_and(|now| now != original)
            } else {
                false
            };
            if disappeared || replaced {
                gone += 1;
                completed.push((base, mask));
                continue;
            }
            let windows = same_client_ping_windows(&conn, base, mask, &client.windows)?;
            client.requires_ping |= !windows.is_empty();
            if windows.is_empty() && !client.requires_ping {
                unsupported += 1;
                completed.push((base, mask));
                continue;
            }
            for window in windows {
                match super::sync_client(&conn, Some(window)) {
                    super::ClientSync::Confirmed => {
                        // Do not attribute a matching nonce/window to an old
                        // recipient if its connection range changed meanwhile.
                        let replaced = if let Some(original) = client.peer_pid {
                            native_client_pid(&conn, base)?.is_some_and(|now| now != original)
                        } else {
                            false
                        };
                        if replaced {
                            gone += 1;
                        } else {
                            confirmed += 1;
                        }
                        completed.push((base, mask));
                        break;
                    }
                    // sync_client folds several errors into Unsupported. Here
                    // strict probing already proved PING support, so neither
                    // Unsupported nor Unanswered is a cleanup acknowledgement.
                    super::ClientSync::Unsupported | super::ClientSync::Unanswered => {
                        conn.get_input_focus()?.reply()?;
                    }
                }
            }
        }
        for key in completed {
            clients.clients.remove(&key);
        }
        if clients.clients.is_empty() {
            break;
        }
        if !pending_announced {
            eprintln!("owned XTEST cleanup pending: waiting for native client acknowledgement");
            pending_announced = true;
        }
        // This exact native owner remains alive while the client is stopped.
        // No task TTL, retry of input, busy loop or replacement worker is used.
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(std::time::Duration::from_secs(1));
    }
    eprintln!(
        "owned XTEST cleanup client drain: confirmed={confirmed}, unsupported={unsupported}, gone={gone}"
    );
    Ok(())
}

fn quiesce_clients(
    display: *mut x11::xlib::Display,
    pair: &DevicePair,
    clients: &mut NativeClients,
) -> Result<x11::xlib::Window> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{BUTTON_RELEASE_EVENT, KEY_RELEASE_EVENT};
    use x11rb::protocol::xtest::ConnectionExt as _;
    collect_clients(pair, clients)?;
    let (conn, screen) = RustConnection::connect(None)?;
    let root = conn.setup().roots[screen].root;
    conn.grab_server()?.check()?;
    let released = (|| -> Result<()> {
        verify_connection_pair(&conn, pair)?;
        conn.xinput_xi_set_client_pointer(x11rb::NONE, pair.pointer)?
            .check()?;
        let keys = conn.query_keymap()?.reply()?.keys;
        let pointer = conn.xinput_xi_query_pointer(root, pair.pointer)?.reply()?;
        // This pair has only its own XTEST slaves. Native state is its ownership
        // authority: release those bits only, never the user's core/other masters.
        for key in 8u16..=255 {
            if keys[usize::from(key / 8)] & (1 << (key % 8)) != 0 {
                conn.xtest_fake_input(KEY_RELEASE_EVENT, key as u8, 0, x11rb::NONE, 0, 0, 0)?
                    .check()?;
            }
        }
        for button in 1u16..=255 {
            if pointer
                .buttons
                .get(usize::from(button / 32))
                .is_some_and(|mask| mask & (1 << (button % 32)) != 0)
            {
                conn.xtest_fake_input(BUTTON_RELEASE_EVENT, button as u8, 0, root, 0, 0, 0)?
                    .check()?;
            }
        }
        conn.get_input_focus()?.reply()?;
        Ok(())
    })();
    let ungrab = (|| -> Result<()> {
        conn.ungrab_server()?.check()?;
        Ok(())
    })();
    released?;
    ungrab?;
    let parking = grabbed(display, || unsafe {
        verify_attachments(display, pair)?;
        let mut attributes: x11::xlib::XSetWindowAttributes = std::mem::zeroed();
        attributes.override_redirect = x11::xlib::True;
        let parking = x11::xlib::XCreateWindow(
            display,
            root.into(),
            0,
            0,
            1,
            1,
            0,
            0,
            x11::xlib::InputOnly as u32,
            std::ptr::null_mut(),
            x11::xlib::CWOverrideRedirect,
            &mut attributes,
        );
        x11::xlib::XMapWindow(display, parking);
        x11::xlib::XLowerWindow(display, parking);
        x11::xinput2::XIWarpPointer(
            display,
            pair.pointer.into(),
            0,
            parking,
            0.0,
            0.0,
            0,
            0,
            0.0,
            0.0,
        );
        let mut mask_byte = 0u8;
        let mut mask = x11::xinput2::XIEventMask {
            deviceid: pair.pointer.into(),
            mask_len: 1,
            mask: &mut mask_byte,
        };
        let grabbed = x11::xinput2::XIGrabDevice(
            display,
            pair.pointer.into(),
            parking,
            x11::xlib::CurrentTime,
            0,
            x11::xinput2::XIGrabModeAsync,
            x11::xinput2::XIGrabModeAsync,
            x11::xlib::False,
            &mut mask,
        );
        if grabbed != x11::xlib::GrabSuccess {
            x11::xlib::XDestroyWindow(display, parking);
            bail!("cannot park owned XTEST pointer before retirement (grab status {grabbed})");
        }
        x11::xinput2::XISetFocus(display, pair.keyboard.into(), 0, x11::xlib::CurrentTime);
        x11::xlib::XSync(display, 0);
        Ok(parking)
    })?;
    Ok(parking)
}

fn close_unrelated_fds(life: RawFd, ready: RawFd) -> Result<()> {
    // The guardian must not retain inherited protocol stdin/stdout, their peer
    // pipe writers, or arbitrary host handles. Stderr remains a diagnostic sink.
    let fds: Vec<RawFd> = std::fs::read_dir("/proc/self/fd")?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter(|fd| *fd != 2 && *fd != life && *fd != ready)
        .collect();
    for fd in fds {
        unsafe { libc::close(fd) };
    }
    Ok(())
}

fn create_pair(display: *mut x11::xlib::Display, name: &str) -> Result<DevicePair> {
    let mut core_pointer = 0;
    ensure!(
        unsafe { x11::xinput2::XIGetClientPointer(display, 0, &mut core_pointer) } != 0,
        "identify original core input pointer"
    );
    let core_pointer = core_pointer
        .try_into()
        .context("core pointer id out of range")?;
    let name_c = std::ffi::CString::new(name)?;
    let mut change = x11::xinput2::XIAnyHierarchyChangeInfo::default();
    unsafe {
        let add = change.add();
        (*add)._type = x11::xinput2::XIAddMaster;
        (*add).name = name_c.as_ptr().cast_mut();
        (*add).send_core = x11::xlib::True;
        (*add).enable = x11::xlib::True;
        ensure!(
            x11::xinput2::XIChangeHierarchy(display, &mut change, 1) == 0,
            "create owned XI master"
        );
        x11::xlib::XSync(display, 0);
    }
    lookup_owned_pair(display, name, core_pointer)?
        .ok_or_else(|| anyhow!("created owned XI pair is not yet observable"))
}

fn lookup_owned_pair(
    display: *mut x11::xlib::Display,
    name: &str,
    core_pointer: u16,
) -> Result<Option<DevicePair>> {
    let devices = super::xi2_query_devices(display)?;
    if !devices
        .iter()
        .any(|(_, _, value)| value.starts_with(&format!("{name} ")))
    {
        return Ok(None);
    }
    let find = |suffix: &str, use_: i32| -> Result<u16> {
        let found: Vec<_> = devices
            .iter()
            .filter(|(_, kind, value)| *kind == use_ && value == &format!("{name}{suffix}"))
            .collect();
        ensure!(
            found.len() == 1,
            "owned XI device is missing or ambiguous: {suffix}"
        );
        found[0]
            .0
            .try_into()
            .context("owned XI device id out of range")
    };
    let pair = DevicePair {
        pointer: find(" pointer", x11::xinput2::XIMasterPointer)?,
        keyboard: find(" keyboard", x11::xinput2::XIMasterKeyboard)?,
        xtest_pointer: find(" XTEST pointer", x11::xinput2::XISlavePointer)?,
        xtest_keyboard: find(" XTEST keyboard", x11::xinput2::XISlaveKeyboard)?,
        core_pointer,
        name: name.to_owned(),
    };
    verify_attachments(display, &pair)?;
    Ok(Some(pair))
}

fn remove_exact_pair(
    display: *mut x11::xlib::Display,
    name: &str,
    expected: Option<&DevicePair>,
) -> Result<()> {
    // A startup reaper may already have reclaimed this crashed worker's pair.
    // Check the complete owner+nonce name while holding the server grab, so an
    // id cannot be reused between attribution and removal.
    unsafe { x11::xlib::XGrabServer(display) };
    let result = (|| -> Result<()> {
        let devices = super::xi2_query_devices(display)?;
        let pointers: Vec<_> = devices
            .iter()
            .filter(|(_, kind, value)| {
                *kind == x11::xinput2::XIMasterPointer && value == &format!("{name} pointer")
            })
            .collect();
        ensure!(pointers.len() <= 1, "ambiguous owned XI master identity");
        let Some((pointer, _, _)) = pointers.first().copied() else {
            ensure!(
                !devices
                    .iter()
                    .any(|(_, _, value)| value.starts_with(&format!("{name} "))),
                "owned XI pair is only partially present"
            );
            return Ok(());
        };
        ensure!(
            expected.is_some(),
            "owned XI pair must be resolved and drained before removal"
        );
        ensure!(
            devices
                .iter()
                .filter(|(_, kind, value)| *kind == x11::xinput2::XIMasterKeyboard
                    && value == &format!("{name} keyboard"))
                .count()
                == 1,
            "owned XI keyboard identity is missing or ambiguous"
        );
        if let Some(expected) = expected {
            ensure!(
                *pointer == i32::from(expected.pointer),
                "owned XI pointer id changed"
            );
            for (id, kind, suffix) in [
                (
                    expected.keyboard,
                    x11::xinput2::XIMasterKeyboard,
                    " keyboard",
                ),
                (
                    expected.xtest_pointer,
                    x11::xinput2::XISlavePointer,
                    " XTEST pointer",
                ),
                (
                    expected.xtest_keyboard,
                    x11::xinput2::XISlaveKeyboard,
                    " XTEST keyboard",
                ),
            ] {
                ensure!(
                    devices
                        .iter()
                        .any(|(device, type_, value)| *device == i32::from(id)
                            && *type_ == kind
                            && value == &format!("{name}{suffix}")),
                    "owned XI device identity changed: {suffix}"
                );
            }
            verify_attachments(display, expected)?;
        }
        let mut change = x11::xinput2::XIAnyHierarchyChangeInfo::default();
        unsafe {
            let remove = change.remove();
            (*remove)._type = x11::xinput2::XIRemoveMaster;
            (*remove).deviceid = *pointer;
            // The pair has only its server-owned XTEST slaves. Floating mode
            // releases/destroys them without reattaching to the user's master.
            (*remove).return_mode = x11::xinput2::XIFloating;
            (*remove).return_pointer = 0;
            (*remove).return_keyboard = 0;
            ensure!(
                x11::xinput2::XIChangeHierarchy(display, &mut change, 1) == 0,
                "remove exact owned XI pair"
            );
            x11::xlib::XSync(display, 0);
        }
        ensure!(
            !super::xi2_query_devices(display)?
                .iter()
                .any(|(_, _, value)| value.starts_with(&format!("{name} "))),
            "owned XI pair remains after removal"
        );
        Ok(())
    })();
    unsafe {
        x11::xlib::XUngrabServer(display);
        x11::xlib::XSync(display, 0);
    }
    result
}

fn verify_attachments(display: *mut x11::xlib::Display, pair: &DevicePair) -> Result<()> {
    let mut count = 0;
    let raw =
        unsafe { x11::xinput2::XIQueryDevice(display, x11::xinput2::XIAllDevices, &mut count) };
    ensure!(
        !raw.is_null() && count >= 0,
        "cannot verify owned XI device attachments"
    );
    let result = (|| -> Result<()> {
        let devices = unsafe { std::slice::from_raw_parts(raw, count as usize) };
        for (device, kind, suffix, attachment) in [
            (
                pair.pointer,
                x11::xinput2::XIMasterPointer,
                " pointer",
                pair.keyboard,
            ),
            (
                pair.keyboard,
                x11::xinput2::XIMasterKeyboard,
                " keyboard",
                pair.pointer,
            ),
            (
                pair.xtest_pointer,
                x11::xinput2::XISlavePointer,
                " XTEST pointer",
                pair.pointer,
            ),
            (
                pair.xtest_keyboard,
                x11::xinput2::XISlaveKeyboard,
                " XTEST keyboard",
                pair.keyboard,
            ),
        ] {
            let expected_name = format!("{}{suffix}", pair.name);
            ensure!(
                devices.iter().any(|info| info.deviceid == i32::from(device)
                    && info._use == kind
                    && info.attachment == i32::from(attachment)
                    && !info.name.is_null()
                    && unsafe { std::ffi::CStr::from_ptr(info.name) }.to_bytes()
                        == expected_name.as_bytes()),
                "owned XI device identity/attachment changed"
            );
        }
        // Do not detach a foreign/physical device that another client attached.
        ensure!(
            !devices
                .iter()
                .any(|info| [i32::from(pair.pointer), i32::from(pair.keyboard)]
                    .contains(&info.attachment)
                    && ![
                        pair.pointer,
                        pair.keyboard,
                        pair.xtest_pointer,
                        pair.xtest_keyboard
                    ]
                    .iter()
                    .any(|id| i32::from(*id) == info.deviceid)),
            "foreign device attached to owned XI pair"
        );
        Ok(())
    })();
    unsafe { x11::xinput2::XIFreeDeviceInfo(raw) };
    result
}

/// Bind only a freshly connected input client. Capture, overlay and arbitrary
/// application clients retain their original pointer and keyboard selection.
pub(super) fn bind_input_connection(conn: &RustConnection) -> Result<()> {
    let Some(context) = INPUT_OWNER.get() else {
        return Ok(());
    };
    let response = native_request(&context.channel, &NativeCommand::Refresh)?;
    let Startup::Ready(pair) = response else {
        bail!("owned XTEST refresh omitted its native identity");
    };
    ensure!(
        pair.name == context.name && pair.core_pointer == context.core_pointer,
        "owned XTEST resource identity changed"
    );
    if let Some(previous) = context.pair.get() {
        ensure!(previous == &pair, "owned XTEST resource generation changed");
    } else {
        context
            .pair
            .set(pair.clone())
            .map_err(|_| anyhow!("owned XTEST initialized concurrently"))?;
    }
    // Short attribution/selection transaction; the server releases this grab
    // if the worker dies. No filesystem or arbitrary application call occurs.
    conn.grab_server()?.check()?;
    let result = (|| -> Result<()> {
        verify_connection_pair(conn, &pair)?;
        // Read core focus before switching this freshly connected client's
        // pointer. Original foreground activation remains the WM/core path.
        let focus = conn.get_input_focus()?.reply()?.focus;
        conn.xinput_xi_set_client_pointer(x11rb::NONE, pair.pointer)?
            .check()?;
        conn.xinput_xi_set_focus(focus, x11rb::CURRENT_TIME, pair.keyboard)?
            .check()?;
        Ok(())
    })();
    let ungrab = (|| -> Result<()> {
        conn.ungrab_server()?.check()?;
        Ok(())
    })();
    result?;
    ungrab?;
    Ok(())
}

fn verify_connection_pair(conn: &RustConnection, pair: &DevicePair) -> Result<()> {
    use x11rb::protocol::xinput::DeviceType;
    let devices = conn.xinput_xi_query_device(0u16)?.reply()?.infos;
    for (id, kind, suffix, attached) in [
        (
            pair.pointer,
            DeviceType::MASTER_POINTER,
            " pointer",
            pair.keyboard,
        ),
        (
            pair.keyboard,
            DeviceType::MASTER_KEYBOARD,
            " keyboard",
            pair.pointer,
        ),
        (
            pair.xtest_pointer,
            DeviceType::SLAVE_POINTER,
            " XTEST pointer",
            pair.pointer,
        ),
        (
            pair.xtest_keyboard,
            DeviceType::SLAVE_KEYBOARD,
            " XTEST keyboard",
            pair.keyboard,
        ),
    ] {
        ensure!(
            devices.iter().any(|info| info.deviceid == id
                && info.type_ == kind
                && info.attachment == attached
                && info.name == format!("{}{suffix}", pair.name).as_bytes()),
            "owned XTEST input identity is no longer valid"
        );
    }
    Ok(())
}

fn native_request(channel: &SharedChannel, command: &NativeCommand) -> Result<Startup> {
    let mut locked = channel
        .lock()
        .map_err(|_| anyhow!("XTEST guardian channel poisoned"))?;
    let io = locked
        .as_mut()
        .ok_or_else(|| anyhow!("XTEST native cleanup owner is unavailable"))?;
    let result = (|| -> Result<Startup> {
        serde_json::to_writer(&mut io.writer, command)?;
        io.writer.write_all(b"\n")?;
        io.writer.flush()?;
        startup(&mut io.reader)
    })();
    if result.is_err() {
        locked.take();
    }
    match result? {
        Startup::Failed(error) => bail!("XTEST native resource request: {error}"),
        response => Ok(response),
    }
}

pub(super) struct CoreMapMirror {
    token: u64,
    channel: SharedChannel,
    restore_private: bool,
}

impl CoreMapMirror {
    pub(super) fn restore_private(&self) -> bool {
        self.restore_private
    }
}

impl Drop for CoreMapMirror {
    fn drop(&mut self) {
        match native_request(&self.channel, &NativeCommand::Restore { token: self.token }) {
            Ok(Startup::Released { restored: false }) => {
                eprintln!("temporary core keymap row has a later author; preserved")
            }
            Ok(Startup::Released { restored: true }) => {}
            Ok(_) => eprintln!("unexpected temporary core keymap cleanup response"),
            Err(error) => {
                // The guardian retains failed cleanup responsibility until EOF.
                eprintln!("XTEST temporary core keymap cleanup: {error:#}");
            }
        }
    }
}

pub(super) fn mirror_temporary_keymap(
    conn: &RustConnection,
    keycode: u8,
    keysyms: &[u32],
) -> Result<Option<CoreMapMirror>> {
    let Some(context) = INPUT_OWNER.get() else {
        return Ok(None);
    };
    let selected = conn.xinput_xi_get_client_pointer(x11rb::NONE)?.reply()?;
    let owned_pointer = context.pair.get().map(|pair| pair.pointer);
    if !selected.set
        || (selected.deviceid != context.core_pointer && Some(selected.deviceid) != owned_pointer)
    {
        return Ok(None);
    }
    let response = native_request(
        &context.channel,
        &NativeCommand::Borrow {
            keycode,
            keysyms: keysyms.to_vec(),
        },
    )?;
    let Startup::Borrowed(token) = response else {
        bail!("unexpected XTEST keymap borrow response");
    };
    Ok(Some(CoreMapMirror {
        token,
        channel: context.channel.clone(),
        restore_private: Some(selected.deviceid) == owned_pointer,
    }))
}

struct CoreRow {
    per_keycode: i32,
    symbols: Vec<x11::xlib::KeySym>,
}

struct BorrowedRow {
    token: u64,
    keycode: u8,
    original: CoreRow,
    authored: Option<CoreRow>,
}

fn core_row(display: *mut x11::xlib::Display, keycode: u8) -> Result<CoreRow> {
    let mut count = 0;
    let raw = unsafe { x11::xlib::XGetKeyboardMapping(display, keycode, 1, &mut count) };
    ensure!(
        !raw.is_null() && count > 0 && count <= 255,
        "cannot read core keycode {keycode}"
    );
    let symbols = unsafe { std::slice::from_raw_parts(raw, count as usize) }.to_vec();
    unsafe { x11::xlib::XFree(raw.cast()) };
    Ok(CoreRow {
        per_keycode: count,
        symbols,
    })
}

fn same_row(left: &CoreRow, right: &CoreRow) -> bool {
    // XKB can change the global return width while an unrelated code changes.
    // Compare semantic trailing NoSymbol padding, not wire request width.
    let trim = |values: &[x11::xlib::KeySym]| {
        values
            .iter()
            .rposition(|value| *value != 0)
            .map_or(0, |index| index + 1)
    };
    left.symbols[..trim(&left.symbols)] == right.symbols[..trim(&right.symbols)]
}

fn write_core_row(display: *mut x11::xlib::Display, keycode: u8, row: &CoreRow) {
    unsafe {
        x11::xlib::XChangeKeyboardMapping(
            display,
            i32::from(keycode),
            row.per_keycode,
            row.symbols.as_ptr().cast_mut(),
            1,
        );
        x11::xlib::XSync(display, 0);
    }
}

fn grabbed<T>(display: *mut x11::xlib::Display, body: impl FnOnce() -> Result<T>) -> Result<T> {
    unsafe { x11::xlib::XGrabServer(display) };
    struct Ungrab(*mut x11::xlib::Display);
    impl Drop for Ungrab {
        fn drop(&mut self) {
            unsafe {
                x11::xlib::XUngrabServer(self.0);
                x11::xlib::XSync(self.0, 0);
            }
        }
    }
    let _ungrab = Ungrab(display);
    body()
}

fn refresh_mapping(
    display: *mut x11::xlib::Display,
    pair: &DevicePair,
    mirrors: &[BorrowedRow],
) -> Result<()> {
    ensure!(
        mirrors.is_empty(),
        "previous XTEST keymap cleanup is still pending"
    );
    grabbed(display, || {
        verify_attachments(display, pair)?;
        // Existing libX11 clones types, symbols, modifiers, actions and compat
        // semantics. The driver's original spare-keycode planner stays intact.
        const XKB_MAP_AND_COMPAT: u32 = (1 << 1) | (1 << 2) | (1 << 5);
        const XKB_ALL_MAP: u32 = 0xff;
        const XKB_CORE_KEYBOARD: u32 = 0x0100;
        let map =
            unsafe { x11::xlib::XkbGetKeyboard(display, XKB_MAP_AND_COMPAT, XKB_CORE_KEYBOARD) };
        ensure!(!map.is_null(), "read current core XKB mapping");
        let result = (|| -> Result<()> {
            unsafe {
                (*map).device_spec = pair.keyboard;
                ensure!(
                    x11::xlib::XkbSetCompatMap(display, 3, map, x11::xlib::False) != 0,
                    "copy core XKB compatibility map"
                );
                ensure!(
                    x11::xlib::XkbSetMap(display, XKB_ALL_MAP, map) != 0,
                    "copy current core XKB map"
                );
                let mut state: x11::xlib::XkbStateRec = std::mem::zeroed();
                ensure!(
                    x11::xlib::XkbGetState(display, XKB_CORE_KEYBOARD, &mut state) == 0,
                    "read current core layout group"
                );
                ensure!(
                    x11::xlib::XkbLockGroup(
                        display,
                        u32::from(pair.keyboard),
                        u32::from(state.group)
                    ) != 0,
                    "copy core layout group"
                );
                x11::xlib::XSync(display, 0);
            }
            Ok(())
        })();
        unsafe { x11::xlib::XkbFreeKeyboard(map, 0, x11::xlib::True) };
        result
    })
}

fn borrow_core_row(
    display: *mut x11::xlib::Display,
    pair: Option<&DevicePair>,
    keycode: u8,
    keysyms: &[u32],
    mirrors: &mut Vec<BorrowedRow>,
    next_token: &mut u64,
) -> Result<u64> {
    ensure!(
        !keysyms.is_empty() && keysyms.len() <= 255,
        "invalid temporary XTEST keymap row"
    );
    ensure!(
        keysyms[0] != 0 && keysyms.iter().all(|symbol| *symbol == keysyms[0]),
        "temporary XTEST remap must repeat one nonzero keysym"
    );
    ensure!(
        !mirrors.iter().any(|row| row.keycode == keycode),
        "core keycode is already borrowed"
    );
    grabbed(display, || {
        if let Some(pair) = pair {
            verify_attachments(display, pair)?;
        }
        let original = core_row(display, keycode)?;
        ensure!(
            original.symbols.iter().all(|symbol| *symbol == 0),
            "core spare keycode changed before input"
        );
        let requested = CoreRow {
            per_keycode: keysyms.len() as i32,
            symbols: keysyms.iter().map(|symbol| (*symbol).into()).collect(),
        };
        let token = *next_token;
        *next_token = next_token
            .checked_add(1)
            .ok_or_else(|| anyhow!("core keymap token exhausted"))?;
        // Hold the original native resource responsibility before mutation.
        let index = mirrors.len();
        mirrors.push(BorrowedRow {
            token,
            keycode,
            original,
            authored: None,
        });
        write_core_row(display, keycode, &requested);
        let applied = (|| -> Result<()> {
            // Record the actual normalized row, never the wire request bytes.
            let authored = core_row(display, keycode)?;
            let expected = x11::xlib::KeySym::from(keysyms[0]);
            let matches = authored.symbols.first().copied() == Some(expected)
                && authored
                    .symbols
                    .iter()
                    .all(|symbol| *symbol == 0 || *symbol == expected);
            mirrors[index].authored = Some(authored);
            ensure!(
                matches,
                "core keymap mirror did not match the requested keysym"
            );
            Ok(())
        })();
        if let Err(error) = applied {
            // This grab has never been released, so no other client could
            // author the row after our write. Roll back here even if readback
            // failed; retain the pending resource if rollback cannot be proven.
            write_core_row(display, keycode, &mirrors[index].original);
            let rollback = core_row(display, keycode)
                .map(|current| same_row(&current, &mirrors[index].original));
            if matches!(rollback, Ok(true)) {
                mirrors.remove(index);
            }
            return Err(error)
                .context("core keymap mirror failed; original-row rollback attempted");
        }
        Ok(token)
    })
}

fn restore_core_row(
    display: *mut x11::xlib::Display,
    token: u64,
    mirrors: &mut Vec<BorrowedRow>,
) -> Result<bool> {
    let Some(index) = mirrors.iter().position(|row| row.token == token) else {
        return Ok(false);
    };
    grabbed(display, || {
        let borrowed = &mirrors[index];
        let current = core_row(display, borrowed.keycode)?;
        if same_row(&current, &borrowed.original) {
            // A failed borrow already attempted rollback under its server
            // grab. Readback may have failed even though rollback completed.
            mirrors.remove(index);
            return Ok(true);
        }
        let authored = borrowed.authored.as_ref().ok_or_else(|| {
            anyhow!("temporary core keymap mutation lacks confirmed authored value")
        })?;
        let restored = same_row(&current, authored);
        if restored {
            write_core_row(display, borrowed.keycode, &borrowed.original);
            ensure!(
                same_row(&core_row(display, borrowed.keycode)?, &borrowed.original),
                "temporary core keymap restoration not confirmed"
            );
        }
        // A different current row belongs to a later author; preserve it.
        mirrors.remove(index);
        Ok(restored)
    })
}

fn restore_all_core_rows(
    display: *mut x11::xlib::Display,
    mirrors: &mut Vec<BorrowedRow>,
) -> Result<()> {
    let tokens: Vec<_> = mirrors.iter().map(|row| row.token).collect();
    let mut failures = Vec::new();
    for token in tokens {
        if let Err(error) = restore_core_row(display, token, mirrors) {
            failures.push(error.to_string());
        }
    }
    ensure!(
        failures.is_empty(),
        "temporary core keymap cleanup failed: {}",
        failures.join("; ")
    );
    Ok(())
}
