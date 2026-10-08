// Renders Clay's web client in WKWebView - the engine the macOS GUI uses - and saves a
// PNG of it, for the native GUI's Phase-0 comparison. It snapshots its own view, so it
// needs no Screen Recording permission, but it must run inside the logged-in desktop
// session (WKWebView needs the window server).
//
//   swiftc -O webview-snapshot.swift -o webview-snapshot
//   webview-snapshot <url> <out.png> <sha256-of-password> [width height delay]
//
// It logs in as the GUI does: AUTO_PASSWORD (the password's SHA-256, accepted from
// loopback only) set before the page's scripts run.
import Cocoa
import WebKit

let args = CommandLine.arguments
guard args.count >= 4, let url = URL(string: args[1]) else {
    FileHandle.standardError.write("usage: webview-snapshot <url> <out.png> <password-sha256> [width height delay]\n".data(using: .utf8)!)
    exit(2)
}
let out = args[2]
let hash = args[3]
let width = args.count > 4 ? Double(args[4]) ?? 800 : 800
let height = args.count > 5 ? Double(args[5]) ?? 600 : 600
let delay = args.count > 6 ? Double(args[6]) ?? 8 : 8

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)

let config = WKWebViewConfiguration()
let setup = """
window.WEBVIEW_MODE = true;
window.WEBVIEW_DEVICE_OVERRIDE = 'desktop';
window.WS_PROTOCOL = 'ws';
window.AUTO_PASSWORD = '\(hash)';
"""
config.userContentController.addUserScript(
    WKUserScript(source: setup, injectionTime: .atDocumentStart, forMainFrameOnly: true))

let frame = NSRect(x: 0, y: 0, width: width, height: height)
let view = WKWebView(frame: frame, configuration: config)
// A window far off screen: WKWebView only renders inside one, but nobody needs to see it.
let window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: width, height: height),
                      styleMask: [.borderless], backing: .buffered, defer: false)
window.contentView = view
window.orderFrontRegardless()
view.load(URLRequest(url: url))

DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
    view.takeSnapshot(with: WKSnapshotConfiguration()) { image, error in
        guard let image = image, let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:]) else {
            FileHandle.standardError.write("snapshot failed: \(String(describing: error))\n".data(using: .utf8)!)
            exit(1)
        }
        do {
            try png.write(to: URL(fileURLWithPath: out))
            print("webview-snapshot: \(out) (\(rep.pixelsWide)x\(rep.pixelsHigh))")
            exit(0)
        } catch {
            FileHandle.standardError.write("can't write \(out): \(error)\n".data(using: .utf8)!)
            exit(1)
        }
    }
}
app.run()
