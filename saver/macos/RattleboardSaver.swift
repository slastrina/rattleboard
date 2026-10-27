// Rattleboard's macOS screen saver: a web view showing the screen saver page (src/saver.html,
// copied into Resources/web), fed once a second with what the Rattleboard app on this Mac is
// showing, from its local feed (http://127.0.0.1:7171/rattleboard/saver, see
// src-tauri/src/saver.rs). With the app not running it hands the page null, and the page
// shows the time and date.
//
// The feed is read here, natively, rather than by the page: the feed refuses web pages.
//
// macOS 14 and later can leave a screen saver running after it's dismissed (legacyScreenSaver
// keeps its views alive), so on "com.apple.screensaver.willstop" the timer stops and the web
// view goes; outside the preview the host process is then ended, as other screen savers do,
// and macOS starts it afresh next time.

import AppKit
import ScreenSaver
import WebKit

@objc(RattleboardSaverView)
final class RattleboardSaverView: ScreenSaverView {
    private var web: WKWebView?
    private var timer: Timer?
    private var busy = false
    private let session: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.timeoutIntervalForRequest = 0.8
        c.timeoutIntervalForResource = 0.8
        c.connectionProxyDictionary = [:]
        c.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: c)
    }()
    private static let feed = URL(string: "http://127.0.0.1:7171/rattleboard/saver")!

    override init?(frame: NSRect, isPreview: Bool) {
        super.init(frame: frame, isPreview: isPreview)
        animationTimeInterval = 1
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        DistributedNotificationCenter.default().addObserver(
            self, selector: #selector(willStop), name: NSNotification.Name("com.apple.screensaver.willstop"), object: nil)
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
    }

    deinit {
        DistributedNotificationCenter.default().removeObserver(self)
    }

    private func makeWeb() {
        guard web == nil else { return }
        let config = WKWebViewConfiguration()
        config.suppressesIncrementalRendering = true
        let w = WKWebView(frame: bounds, configuration: config)
        w.autoresizingMask = [.width, .height]
        w.setValue(false, forKey: "drawsBackground")
        addSubview(w)
        web = w
        let bundle = Bundle(for: RattleboardSaverView.self)
        if let page = bundle.url(forResource: "saver", withExtension: "html", subdirectory: "web") {
            w.loadFileURL(page, allowingReadAccessTo: page.deletingLastPathComponent())
        }
    }

    override func startAnimation() {
        super.startAnimation()
        makeWeb()
        poll()
    }

    override func stopAnimation() {
        super.stopAnimation()
        timer?.invalidate()
        timer = nil
    }

    override func animateOneFrame() {
        poll()
    }

    override var hasConfigureSheet: Bool { false }
    override var configureSheet: NSWindow? { nil }

    /// What the app's board shows now, or null without the app, handed to the page.
    private func poll() {
        guard let web, !busy else { return }
        busy = true
        session.dataTask(with: Self.feed) { data, response, _ in
            var json = "null"
            if let data, (response as? HTTPURLResponse)?.statusCode == 200,
               (try? JSONSerialization.jsonObject(with: data)) is [String: Any],
               let s = String(data: data, encoding: .utf8) {
                json = s
            }
            DispatchQueue.main.async {
                self.busy = false
                web.evaluateJavaScript("window.saverUpdate && window.saverUpdate(\(json))", completionHandler: nil)
            }
        }.resume()
    }

    @objc private func willStop() {
        timer?.invalidate()
        timer = nil
        web?.removeFromSuperview()
        web = nil
        if !isPreview {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { exit(0) }
        }
    }
}
