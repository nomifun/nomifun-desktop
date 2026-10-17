import Foundation
import ServiceManagement

private let settingsLock = NSLock()
private var settingsOpened = false
private let serviceLock = NSLock()
@_silgen_name("nomifun_system_execution_build_identity") private func expectedBuildIdentity() -> UnsafePointer<CChar>

private func unregisterSystemService(_ service: SMAppService) throws {
    if service.status == .notRegistered || service.status == .notFound { return }
    let semaphore = DispatchSemaphore(value: 0)
    var failure: Error?
    service.unregister { error in failure = error; semaphore.signal() }
    guard semaphore.wait(timeout: .now() + 60) == .success else {
        throw NSError(domain: "NomiFunSystemExecution", code: 8, userInfo: [NSLocalizedDescriptionKey: "The native service unregister operation has an unknown result; its process exit was not confirmed"])
    }
    if let failure { throw failure }
}

private func requireSystemService(_ service: SMAppService) throws {
    let initialStatus = service.status
    // A first-use daemon can have no Background Task Management record yet;
    // macOS reports that as .notFound even when its signed bundled plist and
    // executable exist. register() establishes that native record and is the
    // authority for any real registration/signature/approval failure.
    if initialStatus == .notRegistered || initialStatus == .notFound {
        NSLog("NomiFun system execution registration requested; initial status=%ld", initialStatus.rawValue)
        do { try service.register() }
        catch {
            let current = service.status
            let native = error as NSError
            NSLog("NomiFun system execution registration result; status=%ld, native domain=%@, code=%ld", current.rawValue, native.domain, native.code)
            if current != .enabled && current != .requiresApproval { throw error }
        }
        NSLog("NomiFun system execution registration resolved; status=%ld", service.status.rawValue)
    }
    let status = service.status
    switch status {
    case .enabled: break
    case .requiresApproval:
        settingsLock.lock()
        if !settingsOpened { settingsOpened = true; SMAppService.openSystemSettingsLoginItems() }
        settingsLock.unlock()
        throw NSError(domain: "NomiFunSystemExecution", code: 4, userInfo: [NSLocalizedDescriptionKey: "Approve NomiFun administrator execution in System Settings > Login Items & Extensions, then continue this task. The operating system controls this approval."])
    case .notRegistered, .notFound:
        throw NSError(domain: "NomiFunSystemExecution", code: 5, userInfo: [NSLocalizedDescriptionKey: "The native administrator execution service is not registered or is unavailable (initial status \(initialStatus.rawValue), current status \(status.rawValue))."])
    @unknown default:
        throw NSError(domain: "NomiFunSystemExecution", code: 6, userInfo: [NSLocalizedDescriptionKey: "The native administrator execution service has an unknown operating system status (\(status.rawValue))."])
    }
}

private func systemConnection(team: String) -> NSXPCConnection {
    let connection = NSXPCConnection(machServiceName: systemExecutionService, options: .privileged)
    connection.remoteObjectInterface = NSXPCInterface(with: NomiFunSystemExecutionProtocol.self)
    connection.setCodeSigningRequirement(systemExecutionRequirement(team: team, identifiers: [systemExecutionSigningIdentifier]))
    connection.resume()
    return connection
}

private final class SystemExecutionClient {
    var connection: NSXPCConnection
    init() throws {
        serviceLock.lock(); defer { serviceLock.unlock() }
        let team = try systemExecutionTeam()
        let plist = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/LaunchDaemons/\(systemExecutionPlist)")
        guard FileManager.default.fileExists(atPath: plist.path) else {
            throw NSError(domain: "NomiFunSystemExecution", code: 3, userInfo: [NSLocalizedDescriptionKey: "The signed application bundle has no administrator execution service. Reinstall the complete NomiFun application."])
        }
        let service = SMAppService.daemon(plistName: systemExecutionPlist)
        try requireSystemService(service)
        connection = systemConnection(team: team)
        var identity = call(Data("{\"op\":\"identity\"}".utf8))
        var response = (try? JSONSerialization.jsonObject(with: identity)) as? [String: Any]
        if response?["ok"] as? Bool == true,
           let result = response?["result"] as? [String: Any],
           result["build_identity"] as? String != String(cString: expectedBuildIdentity()) {
            let prepared = call(Data("{\"op\":\"prepare_upgrade\"}".utf8))
            let receipt = (try? JSONSerialization.jsonObject(with: prepared)) as? [String: Any]
            guard receipt?["ok"] as? Bool == true, (receipt?["result"] as? [String: Any])?["exact"] as? Bool == true else {
                throw NSError(domain: "NomiFunSystemExecution", code: 9, userInfo: [NSLocalizedDescriptionKey: "The previous native service could not prove cleanup for an upgrade. Its execution result remains unknown."])
            }
            connection.invalidate()
            // Apple's asynchronous completion proves the old daemon stopped
            // before re-registration; no sleep or local authorization ledger.
            try unregisterSystemService(service)
            try requireSystemService(service)
            connection = systemConnection(team: team)
            identity = call(Data("{\"op\":\"identity\"}".utf8))
            response = (try? JSONSerialization.jsonObject(with: identity)) as? [String: Any]
        }
        guard let response,
              response["ok"] as? Bool == true,
              let result = response["result"] as? [String: Any], result["effective_uid"] as? UInt == 0,
              result["build_identity"] as? String == String(cString: expectedBuildIdentity()) else {
            let response = (try? JSONSerialization.jsonObject(with: identity)) as? [String: Any]
            let message = (response?["error"] as? [String: Any])?["message"] as? String ?? "The registered native service did not prove an administrator execution identity"
            throw NSError(domain: "NomiFunSystemExecution", code: 7, userInfo: [NSLocalizedDescriptionKey: message])
        }
    }
    deinit { connection.invalidate() }

    func call(_ request: Data) -> Data {
        let semaphore = DispatchSemaphore(value: 0)
        let lock = NSLock()
        var result: Data?
        func finish(_ data: Data) {
            lock.lock(); defer { lock.unlock() }
            guard result == nil else { return }
            result = data; semaphore.signal()
        }
        guard let proxy = connection.remoteObjectProxyWithErrorHandler({ error in
            finish(systemExecutionFailure("system_connection_lost", "Native administrator connection failed; an in-flight result may be unknown: \(error.localizedDescription)"))
        }) as? NomiFunSystemExecutionProtocol else {
            return systemExecutionFailure("system_connection_lost", "Cannot connect to native administrator execution")
        }
        proxy.perform(request, withReply: finish)
        // This bounds an unavailable XPC service, not a process lifetime. Poll
        // requests are separately bounded; actual children retain supervision.
        guard semaphore.wait(timeout: .now() + 60) == .success else {
            connection.invalidate()
            return systemExecutionFailure("system_result_unknown", "Native administrator execution stopped responding. The connection was closed and its temporary processes are being cleaned up; the operation result is unknown.")
        }
        lock.lock(); defer { lock.unlock() }
        return result ?? systemExecutionFailure("system_result_unknown", "Native administrator execution returned no result")
    }
}

@_cdecl("nomifun_system_client_new")
public func nativeSystemClientNew(_ error: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> UnsafeMutableRawPointer? {
    do { return Unmanaged.passRetained(try SystemExecutionClient()).toOpaque() }
    catch let failure {
        let native = failure as NSError
        let code = native.domain == "NomiFunSystemExecution" && native.code == 4 ? "SYSTEM_EXECUTION_APPROVAL_REQUIRED" : "SYSTEM_EXECUTION_NOT_AVAILABLE"
        let data = try? JSONSerialization.data(withJSONObject: ["code": code, "message": failure.localizedDescription, "details": ["user_code_not_started": true, "cleanup_proven": true]])
        error.pointee = strdup(data.flatMap { String(data: $0, encoding: .utf8) } ?? failure.localizedDescription)
        return nil
    }
}

@_cdecl("nomifun_system_client_call")
public func nativeSystemClientCall(_ pointer: UnsafeMutableRawPointer, _ request: UnsafePointer<UInt8>, _ length: Int) -> UnsafeMutablePointer<CChar>? {
    let client = Unmanaged<SystemExecutionClient>.fromOpaque(pointer).takeUnretainedValue()
    let result = client.call(Data(bytes: request, count: length))
    return result.withUnsafeBytes { data in
        let memory = malloc(result.count + 1)!.assumingMemoryBound(to: CChar.self)
        if let base = data.baseAddress { memcpy(memory, base, result.count) }
        memory[result.count] = 0
        return memory
    }
}

@_cdecl("nomifun_system_client_drop")
public func nativeSystemClientDrop(_ pointer: UnsafeMutableRawPointer) {
    Unmanaged<SystemExecutionClient>.fromOpaque(pointer).release()
}

@_cdecl("nomifun_system_client_free")
public func nativeSystemClientFree(_ pointer: UnsafeMutablePointer<CChar>) { free(pointer) }

@_cdecl("nomifun_system_service_unregister")
public func nativeSystemServiceUnregister() -> UnsafeMutablePointer<CChar>? {
    serviceLock.lock(); defer { serviceLock.unlock() }
    do { try unregisterSystemService(SMAppService.daemon(plistName: systemExecutionPlist)); return nil }
    catch { return strdup(error.localizedDescription) }
}
