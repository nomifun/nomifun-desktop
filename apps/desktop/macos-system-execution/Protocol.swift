import Foundation
import Security

let systemExecutionSigningIdentifier = "com.nomifun.desktop.system-execution"
let systemExecutionService: String = {
    // Product and development App identities own distinct OS registrations;
    // updating a development build must never interrupt a production task.
    if Bundle.main.bundleIdentifier == "com.nomifun.desktop.dev" { return "com.nomifun.desktop.dev.system-execution" }
    let args = ProcessInfo.processInfo.arguments
    if let index = args.firstIndex(of: "--service"), index + 1 < args.count,
       [systemExecutionSigningIdentifier, "com.nomifun.desktop.dev.system-execution"].contains(args[index + 1]) { return args[index + 1] }
    return systemExecutionSigningIdentifier
}()
let systemExecutionPlist = "\(systemExecutionService).plist"

@objc(NomiFunSystemExecutionProtocol) protocol NomiFunSystemExecutionProtocol {
    func perform(_ request: Data, withReply reply: @escaping (Data) -> Void)
}

func systemExecutionFailure(_ code: String, _ message: String, userCodeNotStarted: Bool = false, cleanupProven: Bool = false) -> Data {
    (try? JSONSerialization.data(withJSONObject: ["ok": false, "error": ["code": code, "message": message, "details": ["user_code_not_started": userCodeNotStarted, "cleanup_proven": cleanupProven]]])) ?? Data()
}

// Both ends authenticate signed code before admitting arbitrary system work.
// Team identity comes from the running signed executable, never task input.
func systemExecutionTeam() throws -> String {
    var code: SecCode?
    guard SecCodeCopySelf([], &code) == errSecSuccess, let code else {
        throw NSError(domain: "NomiFunSystemExecution", code: 1, userInfo: [NSLocalizedDescriptionKey: "Cannot inspect application signing identity"])
    }
    var staticCode: SecStaticCode?
    guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else {
        throw NSError(domain: "NomiFunSystemExecution", code: 1, userInfo: [NSLocalizedDescriptionKey: "Cannot inspect static application signing identity"])
    }
    var information: CFDictionary?
    guard SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
          let values = information as? [String: Any],
          let team = values[kSecCodeInfoTeamIdentifier as String] as? String,
          !team.isEmpty, team.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else {
        throw NSError(domain: "NomiFunSystemExecution", code: 2, userInfo: [NSLocalizedDescriptionKey: "Administrator execution requires a NomiFun application signed with an Apple-issued signing identity and Team ID"])
    }
    return team
}

func systemExecutionRequirement(team: String, identifiers: [String]) -> String {
    let ids = identifiers.map { "identifier \"\($0)\"" }.joined(separator: " or ")
    return "anchor apple generic and certificate leaf[subject.OU] = \"\(team)\" and (\(ids))"
}
