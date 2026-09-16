// Native macOS conformance preflight. This is a real WKWebView, not a product fallback.
// AppKit mouse-button/drag failures MUST produce a nonzero exit status.
import AppKit
import WebKit
import CoreGraphics

func probeOption(_ key:String) -> String? {
    guard let index=CommandLine.arguments.firstIndex(of:key), index+1<CommandLine.arguments.count else { return nil }
    return CommandLine.arguments[index+1]
}
func probeURL() -> URL {
    guard let text=probeOption("--fixture-url"), let url=URL(string:text), url.scheme=="http", url.host=="127.0.0.1" else {
        FileHandle.standardError.write(Data("Native probe requires a loopback fixture URL from its runner.\n".utf8))
        exit(2)
    }
    return url
}
func writeProbeReport(_ value:[String:Any],code:Int32) -> Never {
    var report=value
    report["exitCode"]=code
    #if arch(arm64)
    report["architecture"]="arm64"
    #else
    report["architecture"]="x86_64"
    #endif
    report["webkit"]=Bundle(identifier:"com.apple.WebKit")?.infoDictionary?["CFBundleVersion"] ?? "unknown"
    report["probe"]=probeOption("--probe") ?? "input"
    report["transport"]=probeOption("--transport") ?? "appkit"
    guard let path=probeOption("--report"), let data=try? JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]) else { exit(2) }
    do { try data.write(to:URL(fileURLWithPath:path),options:.atomic) } catch { exit(2) }
    exit(code)
}

@MainActor final class MacNativeInputProbe: NSObject, NSApplicationDelegate, WKNavigationDelegate {
    var window: NSWindow!
    var web: WKWebView!
    var running = false
    var locked = false
    var authorized: [(NSEvent.EventType, TimeInterval)] = []
    var blocked = 0
    var nativeEvents:[[String:Any]] = []
    var cursorBefore = NSEvent.mouseLocation
    var cursorAfter = NSEvent.mouseLocation
    var monitor: Any?
    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 180, y: 180, width: 960, height: 720), styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "Nomi WKWebView native input spike"
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        web = WKWebView(frame: NSRect(x: 0, y: 0, width: 960, height: 720), configuration: config)
        web.autoresizingMask = [.width, .height]
        web.navigationDelegate = self
        web.isInspectable = false
        window.contentView!.addSubview(web)
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown,.leftMouseUp,.rightMouseDown,.rightMouseUp,.otherMouseDown,.otherMouseUp,.leftMouseDragged,.rightMouseDragged,.otherMouseDragged,.mouseMoved,.keyDown,.keyUp,.flagsChanged,.scrollWheel,.magnify,.swipe,.rotate,.beginGesture,.endGesture,.pressure]) { [weak self] event in
            guard let self else { return event }
            let match = self.authorized.firstIndex { $0.0 == event.type && abs($0.1-event.timestamp) < 0.000001 }
            let isAuthorized = match != nil
            if let match { self.authorized.remove(at:match) }
            if self.locked && self.nativeEvents.count < 64 {
                self.nativeEvents.append(["type":event.type.rawValue,"window":event.windowNumber,"authorized":isAuthorized,"x":event.locationInWindow.x,"y":event.locationInWindow.y])
            }
            if self.locked && event.windowNumber == self.window.windowNumber && !isAuthorized {
                self.blocked += 1
                return nil
            }
            return event
        }
        if probeOption("--transport") == "pid" && !CGPreflightPostEventAccess() {
            if CommandLine.arguments.contains("--request-event-access") { _ = CGRequestPostEventAccess() }
            finish(["status":"permission_required","passed":false,"reason":"CGPreflightPostEventAccess is false; no target process input was sent"],code:77)
            return
        }
        web.load(URLRequest(url: probeURL()))
        Task { try? await Task.sleep(for: .seconds(35)); finish(["error":"native probe timed out"], code: 2) }
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard !running else { return }
        running = true
        Task { do { try await probe() } catch { finish(["error":String(describing:error)], code:2) } }
    }
    func js(_ source: String) async throws -> Any { try await web.evaluateJavaScript(source) as Any }
    func pause() async { try? await Task.sleep(for: .milliseconds(140)) }
    func point(_ id: String) async throws -> NSPoint {
        let value = try await js("(()=>{const r=document.getElementById('\(id)').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})()") as! [Double]
        return web.convert(NSPoint(x: value[0], y: web.isFlipped ? value[1] : web.bounds.height-value[1]), to: nil)
    }
    func post(_ event: NSEvent, agent: Bool = true) {
        if agent { authorized.append((event.type,event.timestamp)) }
        NSApp.postEvent(event, atStart: false)
    }
    func mouse(_ type:NSEvent.EventType, at point:NSPoint, agent:Bool = true, number:Int = 1) async {
        if probeOption("--transport") == "pid" && agent {
            // Preserve the addressed NSWindow when bridging to the public CG PID transport.
            let addressed = NSEvent.mouseEvent(with:type,location:point,modifierFlags:[],timestamp:ProcessInfo.processInfo.systemUptime,windowNumber:window.windowNumber,context:nil,eventNumber:number,clickCount:1,pressure:type == .leftMouseUp ? 0 : 1)!
            guard let cg=addressed.cgEvent else { return }
            authorized.append((type,addressed.timestamp))
            cg.postToPid(getpid())
        } else {
            let event = NSEvent.mouseEvent(with:type,location:point,modifierFlags:[],timestamp:ProcessInfo.processInfo.systemUptime,windowNumber:window.windowNumber,context:nil,eventNumber:number,clickCount:1,pressure:type == .leftMouseUp ? 0 : 1)!
            post(event,agent:agent)
        }
        await pause()
    }
    func click(_ id:String, agent:Bool = true) async throws {
        let position = try await point(id)
        await mouse(.leftMouseDown,at:position,agent:agent)
        await mouse(.leftMouseUp,at:position,agent:agent)
    }
    func key(_ chars: String, code: UInt16, modifiers: NSEvent.ModifierFlags = []) async {
        for type: NSEvent.EventType in [.keyDown,.keyUp] {
            let event = NSEvent.keyEvent(with:type, location:.zero, modifierFlags:modifiers, timestamp:ProcessInfo.processInfo.systemUptime, windowNumber:window.windowNumber, context:nil, characters:chars, charactersIgnoringModifiers:chars, isARepeat:false, keyCode:code)!
            post(event); await pause()
        }
    }
    func probe() async throws {
        // Navigation completion can precede LaunchServices/window activation.
        // Do not report an input failure against a window which is not ready.
        for _ in 0..<120 {
            if NSApp.isActive && window.isKeyWindow { break }
            await pause()
        }
        guard NSApp.isActive && window.isKeyWindow else {
            finish(["passed":false,"status":"window_not_ready","reason":"The native fixture must be the active key window"],code:2)
            return
        }
        await pause()
        cursorBefore = NSEvent.mouseLocation
        locked = true
        try await click("field")
        let focused = try await js("document.activeElement.id") as? String
        let responder = String(describing: window.firstResponder)
        _ = try await js("for(const type of ['compositionstart','compositionupdate','compositionend'])document.addEventListener(type,e=>smoke.events.push({type,trusted:e.isTrusted,target:e.target.id}),true);true")
        await key("a", code: 0)
        var ime = "unsupported"
        if let client = window.firstResponder as? NSTextInputClient {
            client.setMarkedText("拼音", selectedRange:NSRange(location:2,length:0), replacementRange:NSRange(location:NSNotFound,length:0))
            client.insertText("中文", replacementRange:NSRange(location:NSNotFound,length:0))
            ime = "native NSTextInputClient"
        }
        await pause()
        await key("\t", code: 48)
        try await click("submit")
        let before = try await js("smoke.clicks")
        try await click("submit", agent:false)
        let after = try await js("smoke.clicks")
        let p = try await point("scroller")
        let cg = CGEvent(scrollWheelEvent2Source:nil, units:.pixel, wheelCount:2, wheel1:-150, wheel2:0, wheel3:0)!
        cg.location = NSPoint(x:p.x, y:NSScreen.screens[0].frame.maxY-p.y)
        cg.setIntegerValueField(.mouseEventWindowUnderMousePointer, value:Int64(window.windowNumber))
        cg.setIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent, value:Int64(window.windowNumber))
        let wheel = NSEvent(cgEvent:cg)!
        let wheelWindow = wheel.windowNumber
        web.scrollWheel(with:wheel)
        await pause()
        // The shared fixture only counts captured moves when pointer.buttons is 1.
        let origin = try await point("drag")
        for (index,type): (Int,NSEvent.EventType) in [(0,.leftMouseDown),(1,.leftMouseDragged),(2,.leftMouseDragged),(3,.leftMouseUp)] {
            await mouse(type,at:NSPoint(x:origin.x+Double(index*8),y:origin.y),number:2)
        }
        cursorAfter = NSEvent.mouseLocation
        let result = try await js("({value:field.value,focus:document.activeElement.id,clicks:smoke.clicks,events:smoke.events,scroll:scroller.scrollTop,capturedMoves:smoke.capturedMoves})")
        let page = result as! [String:Any]
        let events = page["events"] as! [[String:Any]]
        let pointerDown = events.filter { $0["type"] as? String == "pointerdown" }
        let checks: [String:Bool] = [
            "click_default_behavior":page["clicks"] as? Int == 1,
            "focus":focused == "field",
            "native_chinese_composition":page["value"] as? String == "a中文" && events.contains { $0["type"] as? String == "compositionend" },
            "all_observed_events_trusted":!events.isEmpty && events.allSatisfy { $0["trusted"] as? Bool == true },
            "mouse_button_state":!pointerDown.isEmpty && pointerDown.allSatisfy { $0["buttons"] as? Int == 1 },
            "pointer_capture_drag":(page["capturedMoves"] as? Int ?? 0) > 0,
            "wheel_default_behavior":(page["scroll"] as? Int ?? 0) > 0,
            "local_input_gate":blocked >= 2 && (before as? Int) == (after as? Int),
            "system_cursor_unchanged":cursorBefore == cursorAfter,
        ]
        let passed = checks.values.allSatisfy { $0 }
        finish(["status":passed ? "passed" : "conformance_failed","passed":passed,"checks":checks,"platform":ProcessInfo.processInfo.operatingSystemVersionString,"scale":window.backingScaleFactor,"window":window.windowNumber,"wheelWindow":wheelWindow,"responder":responder,"ime":ime,"blocked":blocked,"nativeEvents":nativeEvents,"page":result],code:passed ? 0 : 1)
    }
    func finish(_ result:[String:Any],code:Int32) {
        window?.orderOut(nil)
        writeProbeReport(result,code:code)
    }
}

@MainActor final class MacDataStoreProbe: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    let a = UUID(), b = UUID()
    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect:NSRect(x:160,y:180,width:960,height:640),styleMask:[.titled,.closable],backing:.buffered,defer:false)
        window.title = "Nomi WKWebView data isolation probe"
        window.makeKeyAndOrderFront(nil)
        Task { try? await Task.sleep(for:.seconds(45)); finish(["passed":false,"error":"storage probe timed out"],code:2) }
        Task { do { try await run() } catch { finish(["error":String(describing:error)],code:2) } }
    }
    func view(_ id:UUID, x:Double) async throws -> WKWebView {
        let config=WKWebViewConfiguration()
        config.websiteDataStore=WKWebsiteDataStore(forIdentifier:id)
        let web=WKWebView(frame:NSRect(x:x,y:0,width:480,height:640),configuration:config)
        web.isInspectable=false
        window.contentView!.addSubview(web)
        web.load(URLRequest(url:probeURL()))
        for _ in 0..<100 {
            try await Task.sleep(for:.milliseconds(100))
            if !web.isLoading && web.url?.path == "/browser_workspace.html" { return web }
        }
        throw NSError(domain:"Probe",code:1,userInfo:[NSLocalizedDescriptionKey:"navigation timed out"])
    }
    func storage(_ web:WKWebView) async throws -> [String:String] {
        try await web.evaluateJavaScript("({local:localStorage.getItem('nomi-isolation')||'',cookie:document.cookie})") as! [String:String]
    }
    func exercise() async throws -> [String:Any] {
        var first:WKWebView? = try await view(a,x:0)
        let second=try await view(b,x:480)
        _ = try await first!.evaluateJavaScript("localStorage.setItem('nomi-isolation','workspace-a');document.cookie='nomi-isolation=A;max-age=300;SameSite=Lax';true")
        _ = try await second.evaluateJavaScript("localStorage.setItem('nomi-isolation','workspace-b');document.cookie='nomi-isolation=B;max-age=300;SameSite=Lax';true")
        let initialA=try await storage(first!), initialB=try await storage(second)
        let sameView=ObjectIdentifier(second)
        second.isHidden=true; second.frame=NSRect(x:450,y:10,width:490,height:620); second.isHidden=false
        let afterResize=try await storage(second)
        first!.stopLoading();first!.removeFromSuperview();first=nil
        try await Task.sleep(for:.milliseconds(300))
        first=try await view(a,x:0)
        let reopened=try await storage(first!)
        first!.stopLoading();first!.removeFromSuperview();first=nil
        let store=WKWebsiteDataStore(forIdentifier:a)
        await store.removeData(ofTypes:WKWebsiteDataStore.allWebsiteDataTypes(),modifiedSince:Date.distantPast)
        first=try await view(a,x:0)
        let cleared=try await storage(first!), retained=try await storage(second)
        let passed=initialA["local"]=="workspace-a" && initialB["local"]=="workspace-b" && reopened==initialA && cleared["local"]=="" && cleared["cookie"]=="" && retained==initialB && afterResize==initialB && ObjectIdentifier(second)==sameView
        first!.stopLoading();first!.removeFromSuperview();first=nil
        second.stopLoading();second.removeFromSuperview()
        await WKWebsiteDataStore(forIdentifier:b).removeData(ofTypes:WKWebsiteDataStore.allWebsiteDataTypes(),modifiedSince:Date.distantPast)
        return ["passed":passed,"storeA":a.uuidString,"storeB":b.uuidString,"initialA":initialA,"initialB":initialB,"reopenedA":reopened,"clearedA":cleared,"retainedB":retained,"sameViewAfterHideResize":ObjectIdentifier(second)==sameView,"webkit":Bundle(identifier:"com.apple.WebKit")?.infoDictionary?["CFBundleVersion"] ?? "unknown"]
    }
    func run() async throws {
        var report:[String:Any]
        do { report = try await exercise() }
        catch { report = ["passed":false,"error":String(describing:error)] }
        try await Task.sleep(for:.milliseconds(500))
        var cleanupErrors: [String] = []
        for identifier in [a,b] {
            do { try await WKWebsiteDataStore.remove(forIdentifier:identifier) }
            catch { cleanupErrors.append(String(describing:error)) }
        }
        report["cleanupErrors"] = cleanupErrors
        report["removedOwnedStores"] = cleanupErrors.isEmpty
        let passed = report["passed"] as? Bool == true && cleanupErrors.isEmpty
        report["passed"] = passed
        finish(report,code:passed ? 0 : 1)
    }
    func finish(_ value:[String:Any],code:Int32) {
        window?.orderOut(nil)
        writeProbeReport(value,code:code)
    }
}
@main struct Main {
    @MainActor static func main() {
        // Finder, LaunchServices and UI inspection may reopen an app without args.
        // Treat that as a normal usage error, before creating windows or requesting access.
        guard probeOption("--report") != nil,
              ["input","storage","permission"].contains(probeOption("--probe") ?? ""),
              ["appkit","pid"].contains(probeOption("--transport") ?? "") else {
            FileHandle.standardError.write(Data("Launch this diagnostic through run-macos-browser-native-probe.mjs.\n".utf8))
            exit(2)
        }
        if probeOption("--probe") == "permission" {
            let allowed=CGPreflightPostEventAccess()
            writeProbeReport(["status":allowed ? "permission_granted" : "permission_required","eventPostingAllowed":allowed],code:allowed ? 0 : 77)
        }
        _ = probeURL()
        let app=NSApplication.shared
        let delegate: NSApplicationDelegate = probeOption("--probe")=="storage" ? MacDataStoreProbe() : MacNativeInputProbe()
        app.setActivationPolicy(.regular)
        app.delegate=delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}
