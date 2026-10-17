import Foundation
import Darwin

@_silgen_name("nomifun_system_runner_new") private func runnerNew() -> UnsafeMutableRawPointer?
@_silgen_name("nomifun_system_runner_call") private func runnerCall(_ runner: UnsafeMutableRawPointer, _ bytes: UnsafePointer<UInt8>, _ length: Int) -> UnsafeMutablePointer<CChar>?
@_silgen_name("nomifun_system_runner_drop") private func runnerDrop(_ runner: UnsafeMutableRawPointer)
@_silgen_name("nomifun_system_runner_free") private func runnerFree(_ pointer: UnsafeMutablePointer<CChar>)
@_silgen_name("nomifun_system_readonly_worker") private func readonlyWorker() -> Int32

private final class SystemExecutionSession: NSObject, NomiFunSystemExecutionProtocol {
    private let runner: UnsafeMutableRawPointer
    private let lock = NSLock()
    private var disconnected = false
    private var shutdownReceipt: Data?
    private var shutdownWaiters: [(Data) -> Void] = []
    private let prepareUpgrade: () -> Data
    private let callerUID: uid_t
    private let callerHome: String
    private let callerName: String

    init?(callerUID: uid_t, prepareUpgrade: @escaping () -> Data) {
        guard let pointer = runnerNew() else { return nil }
        runner = pointer
        self.callerUID = callerUID
        self.prepareUpgrade = prepareUpgrade
        // This is the authenticated XPC peer, not a task-supplied uid or HOME.
        var account = passwd()
        var found: UnsafeMutablePointer<passwd>?
        let size = max(16384, sysconf(_SC_GETPW_R_SIZE_MAX))
        var buffer = [CChar](repeating: 0, count: size)
        let status = buffer.withUnsafeMutableBufferPointer { bytes in
            getpwuid_r(callerUID, &account, bytes.baseAddress, bytes.count, &found)
        }
        guard status == 0, found != nil else { runnerDrop(pointer); return nil }
        callerHome = String(cString: account.pw_dir)
        callerName = String(cString: account.pw_name)
    }
    deinit { runnerDrop(runner) }

    private func invoke(_ request: Data) -> Data {
        request.withUnsafeBytes { raw in
            guard let base = raw.baseAddress?.assumingMemoryBound(to: UInt8.self), let response = runnerCall(runner, base, request.count) else {
                return systemExecutionFailure("system_result_unknown", "The native process supervisor returned no result")
            }
            defer { runnerFree(response) }
            return Data(bytes: response, count: strlen(response))
        }
    }

    func perform(_ request: Data, withReply reply: @escaping (Data) -> Void) {
        lock.lock(); let closed = disconnected; lock.unlock()
        guard !closed else { reply(systemExecutionFailure("system_connection_lost", "The caller disconnected and its temporary system processes are being cleaned up")); return }
        // Each call retains the session until its Rust call completes, including
        // concurrent poll/cancel/shutdown. No raw runner pointer outlives self.
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            guard var value = (try? JSONSerialization.jsonObject(with: request)) as? [String: Any] else {
                reply(systemExecutionFailure("invalid_system_request", "Expected a native system execution JSON request", userCodeNotStarted: true, cleanupProven: true)); return
            }
            if value["op"] as? String == "prepare_upgrade" { reply(prepareUpgrade()); return }
            if value["op"] as? String == "start" {
                var env = value["env"] as? [String: String] ?? [:]
                // Preserve explicit task configuration and the actual logged-in
                // user's home. Never redirect user tools into /var/root.
                if env["HOME"] == nil { env["HOME"] = callerHome }
                if env["USER"] == nil { env["USER"] = callerName }
                if env["LOGNAME"] == nil { env["LOGNAME"] = callerName }
                env["NOMIFUN_CALLER_UID"] = String(callerUID)
                env["NOMIFUN_EXECUTION_IDENTITY"] = "administrator"
                value["env"] = env
            }
            guard let data = try? JSONSerialization.data(withJSONObject: value) else {
                reply(systemExecutionFailure("invalid_system_request", "Cannot encode native system execution request", userCodeNotStarted: true, cleanupProven: true)); return
            }
            reply(invoke(data))
        }
    }

    func disconnect(completion: @escaping (Data) -> Void) {
        lock.lock()
        if let receipt = shutdownReceipt { lock.unlock(); completion(receipt); return }
        shutdownWaiters.append(completion)
        if disconnected { lock.unlock(); return }
        disconnected = true
        lock.unlock()
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            let report = invoke(Data("{\"op\":\"shutdown\"}".utf8))
            // A dead caller cannot receive a receipt. Retain truthful native
            // cleanup evidence in the system log instead of claiming delivery.
            let result = ((try? JSONSerialization.jsonObject(with: report)) as? [String: Any])?["result"] as? [String: Any]
            NSLog("NomiFun system execution caller uid=%u disconnected; cleanup exact=%@; temporary sessions=%lu", callerUID, String(describing: result?["exact"] ?? "unknown"), (result?["sessions"] as? [Any])?.count ?? 0)
            lock.lock(); shutdownReceipt = report; let waiters = shutdownWaiters; shutdownWaiters.removeAll(); lock.unlock()
            for waiter in waiters { waiter(report) }
        }
    }
}

private final class SystemExecutionListener: NSObject, NSXPCListenerDelegate {
    private let requirement: String
    private let lock = NSLock()
    private var sessions: [ObjectIdentifier: SystemExecutionSession] = [:]
    private var stopping = false
    init(team: String) {
        requirement = systemExecutionRequirement(team: team, identifiers: [systemExecutionService == "com.nomifun.desktop.dev.system-execution" ? "com.nomifun.desktop.dev" : "com.nomifun.desktop"])
    }
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        connection.setCodeSigningRequirement(requirement)
        lock.lock(); let stopped = stopping; lock.unlock()
        guard !stopped, let session = SystemExecutionSession(callerUID: connection.effectiveUserIdentifier, prepareUpgrade: { [weak self] in
            self?.shutdown() ?? systemExecutionFailure("system_result_unknown", "Native service upgrade cleanup is unavailable")
        }) else { return false }
        let key = ObjectIdentifier(connection)
        lock.lock()
        if stopping { lock.unlock(); return false }
        sessions[key] = session; lock.unlock()
        connection.exportedInterface = NSXPCInterface(with: NomiFunSystemExecutionProtocol.self)
        connection.exportedObject = session
        let disconnected: () -> Void = { [weak self, weak session] in
            session?.disconnect { [weak self] receipt in
                guard let self else { return }
                let response = (try? JSONSerialization.jsonObject(with: receipt)) as? [String: Any]
                guard response?["ok"] as? Bool == true, (response?["result"] as? [String: Any])?["exact"] as? Bool == true else {
                    // Keep unresolved native ownership in this live service;
                    // an empty connection map must never erase cleanup loss.
                    return
                }
                self.lock.lock(); self.sessions.removeValue(forKey: key); self.lock.unlock()
            }
        }
        connection.interruptionHandler = disconnected
        connection.invalidationHandler = disconnected
        connection.resume()
        return true
    }
    func shutdown() -> Data {
        lock.lock(); stopping = true; let active = Array(sessions.values); lock.unlock()
        let group = DispatchGroup()
        let receiptsLock = NSLock()
        var receipts: [[String: Any]] = []
        for session in active {
            group.enter(); session.disconnect { data in
                let receipt = ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any]) ?? [:]
                receiptsLock.lock(); receipts.append(receipt); receiptsLock.unlock()
                group.leave()
            }
        }
        group.wait()
        let exact = receipts.allSatisfy { receipt in
            receipt["ok"] as? Bool == true && (receipt["result"] as? [String: Any])?["exact"] as? Bool == true
        }
        return (try? JSONSerialization.data(withJSONObject: ["ok": true, "result": ["exact": exact, "connections": receipts]])) ?? systemExecutionFailure("system_result_unknown", "Cannot encode native service cleanup evidence")
    }
}

@main struct NativeSystemExecutionHelper {
    static func main() {
        guard geteuid() == 0 else { fputs("NomiFun administrator execution must be started by its registered system service.\n", stderr); exit(77) }
        if Array(ProcessInfo.processInfo.arguments.dropFirst()) == ["--readonly-acquire"] { exit(readonlyWorker()) }
        do {
            let delegate = SystemExecutionListener(team: try systemExecutionTeam())
            let listener = NSXPCListener(machServiceName: systemExecutionService)
            listener.delegate = delegate
            signal(SIGTERM, SIG_IGN)
            signal(SIGINT, SIG_IGN)
            let term = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .global())
            let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .global())
            let stop = { listener.suspend(); let report = delegate.shutdown(); NSLog("NomiFun system service is stopping; cleanup receipt bytes=%lu", report.count); exit(0) }
            term.setEventHandler(handler: stop); interrupt.setEventHandler(handler: stop)
            term.resume(); interrupt.resume(); listener.resume()
            withExtendedLifetime((delegate, term, interrupt)) { RunLoop.current.run() }
        } catch {
            NSLog("NomiFun administrator execution cannot start: %@", error.localizedDescription)
            exit(78)
        }
    }
}
