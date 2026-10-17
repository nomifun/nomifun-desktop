// Test fixture metadata only. No activation, input, AX operation or image read.
import AppKit
import CoreGraphics
import Foundation

func fail(_ code: String) -> Never {
    print("{\"error_code\":\"\(code)\",\"ui_effects\":false}")
    exit(1)
}

guard CommandLine.arguments.count == 3,
      let pid = Int32(CommandLine.arguments[1]), pid > 1,
      CommandLine.arguments[2] == "com.nomifun.desktop" else { fail("INVALID_FIXTURE_CALLER") }
guard let app = NSRunningApplication(processIdentifier: pid),
      app.bundleIdentifier == CommandLine.arguments[2],
      let url = app.bundleURL,
      url.resolvingSymlinksInPath().path == "/Applications/NomiFun Global OS Acceptance.app" else {
    fail("CALLER_IS_NOT_DEDICATED_ACCEPTANCE_APP")
}
guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
    fail("WINDOW_METADATA_UNAVAILABLE")
}
var candidates: [[String: Any]] = []
for window in windows {
    // Do not retain or print owner names, titles, or any other app's metadata.
    guard (window[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid,
          (window[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
          let number = (window[kCGWindowNumber as String] as? NSNumber)?.uint32Value, number > 0,
          let bounds = window[kCGWindowBounds as String] as? [String: Any],
          let x = (bounds["X"] as? NSNumber)?.doubleValue,
          let y = (bounds["Y"] as? NSNumber)?.doubleValue,
          let width = (bounds["Width"] as? NSNumber)?.doubleValue,
          let height = (bounds["Height"] as? NSNumber)?.doubleValue,
          x.isFinite, y.isFinite, width.isFinite, height.isFinite,
          width >= 880, height >= 600,
          (window[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0 > 0 else { continue }
    candidates.append(["window_id": number, "layer": 0, "x": x, "y": y, "width": width, "height": height])
}
guard candidates.count == 1 else { fail("ACCEPTANCE_MAIN_WINDOW_NOT_UNIQUE") }
let report: [String: Any] = [
    "schema": "nomifun.desktop-acceptance-window-probe.v1",
    "pid": pid, "bundle_identifier": CommandLine.arguments[2],
    "app_path": url.resolvingSymlinksInPath().path,
    "window": candidates[0], "metadata_only": true, "ui_effects": false,
]
guard let data = try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]),
      let json = String(data: data, encoding: .utf8) else { fail("METADATA_ENCODING_FAILED") }
print(json)
