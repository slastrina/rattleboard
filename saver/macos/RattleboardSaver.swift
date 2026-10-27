// Rattleboard's macOS screen saver. It draws the split-flap board natively, with Core
// Animation layers, from what the Rattleboard app on this Mac is showing (its local feed,
// http://127.0.0.1:7171/rattleboard/saver, see src-tauri/src/saver.rs), or the time and date
// when the app isn't running.
//
// Native rather than a web page: inside macOS's screen saver host (legacyScreenSaver) a web
// view's window never counts as visible, so WebKit suspends the page (no animation, then a
// black screen). Core Animation runs in the window server and isn't held back.
//
// It follows the page the Windows screen saver uses (src/saver.js): the same drum order,
// colours and finishes as src/shared/split-flap.js, the same clock, and the same burn-in care:
// the board drifts a few points every minute, and every five minutes a wave of colour sweeps
// it and every flap turns over its whole drum.

import AppKit
import CoreText
import QuartzCore
import ScreenSaver
import os

private let log = Logger(subsystem: "io.github.slastrina.rattleboard.saver", category: "saver")

// MARK: - Flaps

private enum Flaps {
    static let symbols: [Int: String] = [37: "!", 38: "@", 39: "#", 40: "$", 41: "(", 42: ")", 44: "-", 46: "+", 47: "&", 48: "=",
                                         49: ";", 50: ":", 52: "'", 53: "\"", 54: "%", 55: ",", 56: ".", 59: "/", 60: "?", 62: "°"]
    static let colours: [Int: NSColor] = [63: rgb(0xd93a2f), 64: rgb(0xec7a26), 65: rgb(0xf2c230), 66: rgb(0x2f9a52),
                                          67: rgb(0x2b6bd1), 68: rgb(0x7d4cc2), 69: rgb(0xf3f1ec), 70: rgb(0x0d0d0d)]
    /// Drum order: blank, A–Z, 1–9, 0, punctuation, colour flaps.
    static let drum: [Int] = Array(0...36) + symbols.keys.sorted() + Array(63...71)
    static let index: [Int: Int] = Dictionary(uniqueKeysWithValues: drum.enumerated().map { ($1, $0) })

    static func glyph(_ c: Int) -> String {
        switch c {
        case 1...26: return String(UnicodeScalar(UInt8(64 + c)))
        case 27...35: return String(c - 26)
        case 36: return "0"
        default: return symbols[c] ?? ""
        }
    }

    static func code(_ ch: Character) -> Int {
        let s = String(ch).uppercased()
        guard let u = s.unicodeScalars.first else { return 0 }
        switch u.value {
        case 65...90: return Int(u.value) - 64
        case 49...57: return Int(u.value) - 48 + 26
        case 48: return 36
        default: return symbols.first { $0.value == s }?.key ?? 0
        }
    }
}

private func rgb(_ v: Int, _ a: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((v >> 16) & 255) / 255, green: CGFloat((v >> 8) & 255) / 255, blue: CGFloat(v & 255) / 255, alpha: a)
}

private struct Finish {
    let frame, tile, ink, seam, fill: NSColor
    static let black = Finish(frame: rgb(0x0a0a0a), tile: rgb(0x1c1c1c), ink: rgb(0xf0eee8), seam: NSColor(white: 0, alpha: 0.9), fill: rgb(0xf0eee8))
    static let white = Finish(frame: rgb(0xecebe6), tile: rgb(0xfbfaf7), ink: rgb(0x1c1c1c), seam: NSColor(white: 0, alpha: 0.22), fill: rgb(0x1c1c1c))
}

typealias Grid = [[Int]]

private func blankGrid(_ rows: Int, _ cols: Int) -> Grid { Array(repeating: Array(repeating: 0, count: cols), count: rows) }

// MARK: - The board

/// One flap's faces, drawn once per size and finish.
private final class Faces {
    private var cache: [Int: CGImage] = [:]
    let w: CGFloat, h: CGFloat, scale: CGFloat, finish: Finish, font: NSFont

    init(w: CGFloat, h: CGFloat, scale: CGFloat, finish: Finish) {
        self.w = w; self.h = h; self.scale = scale; self.finish = finish
        let size = (h * 0.6).rounded()
        font = NSFont(name: "BarlowSemiCondensed-SemiBold", size: size) ?? NSFont.systemFont(ofSize: size, weight: .semibold)
    }

    func image(_ code: Int) -> CGImage? {
        if let i = cache[code] { return i }
        let pw = max(1, Int((w * scale).rounded())), ph = max(1, Int((h * scale).rounded()))
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let ctx = CGContext(data: nil, width: pw, height: ph, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        ctx.scaleBy(x: scale, y: scale)
        let fill = code == 71 ? finish.fill : (Flaps.colours[code] ?? finish.tile)
        ctx.setFillColor(fill.cgColor)
        ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
        let text = Flaps.glyph(code)
        if code < 63, !text.isEmpty {
            let attr = NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: finish.ink])
            let line = CTLineCreateWithAttributedString(attr)
            let width = CTLineGetTypographicBounds(line, nil, nil, nil)
            ctx.textPosition = CGPoint(x: (w - CGFloat(width)) / 2, y: (h - font.capHeight) / 2)
            CTLineDraw(line, ctx)
        }
        let img = ctx.makeImage()
        cache[code] = img
        return img
    }
}

private final class Tile {
    let root = CALayer(), top = CALayer(), bottom = CALayer(), flapTop = CALayer(), flapBottom = CALayer()
    let shade = CAGradientLayer(), seam = CALayer()
    var cur = 0, target = 0
    var nextAt: CFTimeInterval = 0
    let jitter = 0.9 + Double.random(in: 0...0.2)
    let r: Int, c: Int

    init(r: Int, c: Int) {
        self.r = r; self.c = c
        for l in [top, bottom, flapTop, flapBottom] {
            l.contentsGravity = .resize
            root.addSublayer(l)
        }
        // Top halves show the upper half of a face image, bottoms the lower (in a layer's
        // unit coordinates y goes up).
        top.contentsRect = CGRect(x: 0, y: 0.5, width: 1, height: 0.5)
        flapTop.contentsRect = top.contentsRect
        bottom.contentsRect = CGRect(x: 0, y: 0, width: 1, height: 0.5)
        flapBottom.contentsRect = bottom.contentsRect
        flapTop.anchorPoint = CGPoint(x: 0.5, y: 0)      // hinged on its lower edge
        flapBottom.anchorPoint = CGPoint(x: 0.5, y: 1)   // hinged on its upper edge
        flapTop.isHidden = true; flapBottom.isHidden = true
        flapTop.zPosition = 2; flapBottom.zPosition = 2
        shade.zPosition = 1; seam.zPosition = 3
        shade.colors = [NSColor(white: 0, alpha: 0.22).cgColor, NSColor(white: 0, alpha: 0).cgColor]
        shade.startPoint = CGPoint(x: 0.5, y: 1); shade.endPoint = CGPoint(x: 0.5, y: 0.45)
        root.addSublayer(shade); root.addSublayer(seam)
        root.masksToBounds = true
    }

    func layout(_ frame: CGRect, finish: Finish) {
        root.frame = frame
        let w = frame.width, h = frame.height
        root.cornerRadius = max(1.5, w * 0.08)
        var t = CATransform3DIdentity
        t.m34 = -1 / (h * 5)
        root.sublayerTransform = t
        top.frame = CGRect(x: 0, y: h / 2, width: w, height: h / 2)
        bottom.frame = CGRect(x: 0, y: 0, width: w, height: h / 2)
        for f in [flapTop, flapBottom] {
            f.bounds = CGRect(x: 0, y: 0, width: w, height: h / 2)
            f.position = CGPoint(x: w / 2, y: h / 2)
        }
        shade.frame = bottom.frame
        seam.frame = CGRect(x: 0, y: h / 2 - 0.5, width: w, height: 1)
        seam.backgroundColor = finish.seam.cgColor
    }

    /// Straight to where it's going.
    func settle(_ faces: Faces) {
        cur = target
        nextAt = 0
        let img = faces.image(Flaps.drum[cur])
        top.contents = img; bottom.contents = img
        flapTop.isHidden = true; flapBottom.isHidden = true
    }

    /// One flap over, taking `d` seconds.
    func step(_ faces: Faces, _ d: CFTimeInterval) {
        let a = faces.image(Flaps.drum[cur]), ni = (cur + 1) % Flaps.drum.count, b = faces.image(Flaps.drum[ni])
        cur = ni
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        top.contents = b; bottom.contents = a
        flapTop.contents = a; flapBottom.contents = b
        flapTop.isHidden = false; flapBottom.isHidden = false
        // Where they end up: the top flap folded away, the bottom one down flat.
        flapTop.transform = CATransform3DMakeRotation(-.pi / 2, 1, 0, 0)
        flapBottom.transform = CATransform3DIdentity
        CATransaction.commit()

        let half = d / 2
        let fall = CABasicAnimation(keyPath: "transform.rotation.x")
        fall.fromValue = 0
        fall.toValue = -Double.pi / 2
        fall.duration = half
        fall.timingFunction = CAMediaTimingFunction(controlPoints: 0.55, 0, 1, 0.7)
        flapTop.add(fall, forKey: "flip")
        let land = CABasicAnimation(keyPath: "transform.rotation.x")
        land.fromValue = Double.pi / 2
        land.toValue = 0
        land.beginTime = CACurrentMediaTime() + half
        land.duration = half
        land.fillMode = .backwards
        land.timingFunction = CAMediaTimingFunction(name: .easeOut)
        flapBottom.add(land, forKey: "flip")
    }

    /// Once the last step has landed.
    func rest(_ faces: Faces) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let img = faces.image(Flaps.drum[cur])
        bottom.contents = img; top.contents = img
        flapTop.isHidden = true; flapBottom.isHidden = true
        CATransaction.commit()
    }
}

/// How a message turns over: when each flap starts, in seconds from now.
private enum Wave {
    case classic, column, reverseColumn, edgesToCenter, row, diagonal, random

    static func named(_ s: String?) -> Wave {
        switch s {
        case "column": return .column
        case "reverse-column": return .reverseColumn
        case "edges-to-center": return .edgesToCenter
        case "row": return .row
        case "diagonal": return .diagonal
        case "random": return .random
        default: return .classic
        }
    }

    func start(r: Int, c: Int, cols: Int, gap: Double, order: Int) -> Double {
        let group: Int
        switch self {
        case .classic: return Double(c) * 0.012 + Double.random(in: 0...0.16)
        case .column: group = c
        case .reverseColumn: group = cols - 1 - c
        case .edgesToCenter: group = min(c, cols - 1 - c)
        case .row: group = r
        case .diagonal: group = r + c
        case .random: group = order
        }
        return Double(group) * gap + Double.random(in: 0...min(0.03, gap / 2))
    }
}

private struct View {
    let grid: Grid, rows: Int, cols: Int, finish: String, seq: Int, wave: String?
}

// MARK: - The screen saver

@objc(RattleboardSaverView)
final class RattleboardSaverView: ScreenSaverView {
    private static let feed = URL(string: "http://127.0.0.1:7171/rattleboard/saver")!
    private static let registerFont: Void = {
        if let url = Bundle(for: RattleboardSaverView.self).url(forResource: "BarlowSemiCondensed-SemiBold", withExtension: "ttf") {
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        }
    }()

    private let stage = CALayer(), board = CALayer(), credit = CATextLayer()
    private var tiles: [Tile] = []
    private var faces: Faces?
    private var rows = 0, cols = 0, finishName = "", builtFor = CGSize.zero
    private let stepSeconds: CFTimeInterval = 0.062
    private var moving = false
    /// Dismissed: macOS 14 and later can keep calling a screen saver after it's gone (and never
    /// call stopAnimation), so "com.apple.screensaver.willstop" puts it to rest until the
    /// next startAnimation.
    private var resting = false

    private var view: View?
    private var heard = Date.distantPast
    private var shown = ""
    private var wiping = false
    private var started = Date()
    private var lastWipe = Date.distantPast
    private var lastPoll = Date.distantPast
    private var lastDrift = Date.distantPast
    private var driftStep = Double.random(in: 0...1000)
    private var busy = false
    private var waveIndex = Int.random(in: 0..<5)
    private let session: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.timeoutIntervalForRequest = 0.8
        c.timeoutIntervalForResource = 0.8
        c.connectionProxyDictionary = [:]
        c.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: c)
    }()

    override init?(frame: NSRect, isPreview: Bool) {
        super.init(frame: frame, isPreview: isPreview)
        _ = Self.registerFont
        animationTimeInterval = isPreview ? 0.5 : 1.0 / 30
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        stage.anchorPoint = CGPoint(x: 0.5, y: 0.5)
        stage.addSublayer(board)
        credit.string = "iokig.com"
        credit.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        credit.foregroundColor = NSColor(srgbRed: 240 / 255, green: 238 / 255, blue: 232 / 255, alpha: 0.28).cgColor
        credit.alignmentMode = .right
        stage.addSublayer(credit)
        layer?.addSublayer(stage)
        DistributedNotificationCenter.default().addObserver(
            self, selector: #selector(willStop), name: NSNotification.Name("com.apple.screensaver.willstop"), object: nil)
        log.info("init preview=\(isPreview, privacy: .public) size=\(Int(frame.width), privacy: .public)x\(Int(frame.height), privacy: .public)")
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
    }

    deinit {
        DistributedNotificationCenter.default().removeObserver(self)
    }

    @objc private func willStop() {
        log.info("willstop preview=\(self.isPreview, privacy: .public)")
        if !isPreview { rest() }
    }

    private func rest() {
        resting = true
        moving = false
        stage.removeAllAnimations()
    }

    override var hasConfigureSheet: Bool { false }
    override var configureSheet: NSWindow? { nil }

    override func startAnimation() {
        super.startAnimation()
        log.info("start preview=\(self.isPreview, privacy: .public)")
        resting = false
        started = Date()
        lastWipe = .distantPast
        render(force: true)
        poll()
    }

    override func stopAnimation() {
        super.stopAnimation()
        rest()
        log.info("stop preview=\(self.isPreview, privacy: .public)")
    }

    override func animateOneFrame() {
        guard !resting else { return }
        let now = Date()
        if bounds.size != builtFor { shown = "" }
        if now.timeIntervalSince(lastPoll) >= 1 { poll() }
        if now.timeIntervalSince(lastDrift) >= 60 { drift() }
        if !isPreview && !wiping {
            // A first wave soon after starting, so every flap turns over at least once; then
            // one every five minutes.
            let due = lastWipe == .distantPast ? now.timeIntervalSince(started) >= 20 : now.timeIntervalSince(lastWipe) >= 300
            if due { wipe() }
        }
        render(force: false)
        tick()
    }

    // MARK: the app's board

    private func poll() {
        lastPoll = Date()
        guard !busy else { return }
        busy = true
        session.dataTask(with: Self.feed) { [weak self] data, response, _ in
            var v: View?
            if let data, (response as? HTTPURLResponse)?.statusCode == 200,
               let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
               let display = o["display"] as? [[Int]] {
                let rows = (o["rows"] as? Int) ?? display.count
                let cols = (o["cols"] as? Int) ?? (display.first?.count ?? 0)
                let wave = (o["transition"] as? [String: Any])?["strategy"] as? String
                if (1...30).contains(rows) && (1...60).contains(cols) {
                    v = View(grid: display, rows: rows, cols: cols, finish: (o["finish"] as? String) ?? "black", seq: (o["seq"] as? Int) ?? 0, wave: wave)
                }
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.busy = false
                self.view = v
                if v != nil { self.heard = Date() }
            }
        }.resume()
    }

    // MARK: what to show

    private func clockGrid() -> Grid {
        var g = blankGrid(6, 22)
        let now = Date(), cal = Calendar.current, en = Locale(identifier: "en")
        let fmt = DateFormatter()
        fmt.locale = en
        fmt.dateFormat = "EEEE"
        let day = fmt.string(from: now)
        fmt.dateFormat = "MMMM"
        let month = fmt.string(from: now)
        let date = "\(cal.component(.day, from: now)) \(month) \(cal.component(.year, from: now))"
        let h = cal.component(.hour, from: now), m = String(format: "%02d", cal.component(.minute, from: now))
        let twelve = (DateFormatter.dateFormat(fromTemplate: "j", options: 0, locale: .current) ?? "h").contains("a")
        let time = twelve ? "\(h % 12 == 0 ? 12 : h % 12):\(m) \(h < 12 ? "AM" : "PM")" : String(format: "%02d", h) + ":" + m
        func line(_ r: Int, _ text: String) {
            let t = Array(text.prefix(22)), at = (22 - t.count) / 2
            for (i, ch) in t.enumerated() { g[r][at + i] = Flaps.code(ch) }
        }
        line(1, day); line(2, date); line(4, time)
        return g
    }

    /// The app's board while it's running and showing something, otherwise the clock.
    private func current() -> (grid: Grid, rows: Int, cols: Int, finish: String, key: String, wave: String?) {
        if let v = view, Date().timeIntervalSince(heard) < 6, v.grid.contains(where: { $0.contains { $0 != 0 } }) {
            var g = blankGrid(v.rows, v.cols)
            for r in 0..<min(v.rows, v.grid.count) { for c in 0..<min(v.cols, v.grid[r].count) { g[r][c] = v.grid[r][c] } }
            return (g, v.rows, v.cols, v.finish == "white" ? "white" : "black", "app:\(v.seq)", v.wave)
        }
        return (clockGrid(), 6, 22, "black", "clock", "column")
    }

    private func render(force: Bool) {
        guard !wiping else { return }
        let b = current()
        let key = "\(b.key)|\(b.grid)|\(b.rows)x\(b.cols)|\(b.finish)"
        guard force || key != shown else { return }
        shown = key
        put(b.grid, rows: b.rows, cols: b.cols, finish: b.finish, wave: Wave.named(b.wave), gap: b.key == "clock" ? 0.04 : 0.12)
    }

    // MARK: drawing

    private func build(rows: Int, cols: Int, finish: String) {
        let f = finish == "white" ? Finish.white : Finish.black
        let W = bounds.width, H = bounds.height
        let m = max(isPreview ? 6 : 24, (min(W, H) * 0.1).rounded())
        let minPad: CGFloat = isPreview ? 4 : 22, below: CGFloat = isPreview ? 0 : 40
        func sizes(_ w: CGFloat) -> (h: CGFloat, gx: CGFloat, gy: CGFloat, pad: CGFloat) {
            ((w * 1.42).rounded(), max(1, (w * 0.17).rounded()), max(1, (w * 0.3).rounded()), max(minPad, (w * 1.15).rounded()))
        }
        func fits(_ w: CGFloat) -> Bool {
            let s = sizes(w)
            return CGFloat(cols) * w + CGFloat(cols - 1) * s.gx + 2 * s.pad <= W - 2 * m
                && CGFloat(rows) * s.h + CGFloat(rows - 1) * s.gy + 2 * s.pad + below <= H - 2 * m
        }
        var lo: CGFloat = 2, hi: CGFloat = 1000
        while lo < hi { let mid = ((lo + hi) / 2).rounded(.up); if fits(mid) { lo = mid } else { hi = mid - 1 } }
        let w = lo, s = sizes(w)
        let bw = CGFloat(cols) * w + CGFloat(cols - 1) * s.gx + 2 * s.pad
        let bh = CGFloat(rows) * s.h + CGFloat(rows - 1) * s.gy + 2 * s.pad
        let scale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        let fs = min(18, max(9, (H / 1080 * 11).rounded())), creditH = isPreview ? 0 : fs + 14

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        board.sublayers?.forEach { $0.removeFromSuperlayer() }
        tiles = []
        stage.bounds = CGRect(x: 0, y: 0, width: bw, height: bh + creditH)
        stage.position = CGPoint(x: W / 2, y: H / 2)
        board.frame = CGRect(x: 0, y: creditH, width: bw, height: bh)
        board.backgroundColor = f.frame.cgColor
        board.cornerRadius = w * 0.4
        board.shadowOpacity = 0.4
        board.shadowRadius = 24
        board.shadowOffset = CGSize(width: 0, height: -12)
        credit.fontSize = fs
        credit.contentsScale = scale
        credit.frame = CGRect(x: 0, y: 0, width: bw, height: fs + 4)
        credit.isHidden = isPreview
        let faces = Faces(w: w, h: s.h, scale: scale, finish: f)
        self.faces = faces
        for r in 0..<rows {
            for c in 0..<cols {
                let t = Tile(r: r, c: c)
                let x = s.pad + CGFloat(c) * (w + s.gx), y = bh - s.pad - CGFloat(r + 1) * s.h - CGFloat(r) * s.gy
                t.layout(CGRect(x: x, y: y, width: w, height: s.h), finish: f)
                for l in [t.root, t.top, t.bottom, t.flapTop, t.flapBottom, t.shade, t.seam] { l.contentsScale = scale }
                board.addSublayer(t.root)
                tiles.append(t)
                t.settle(faces)
            }
        }
        CATransaction.commit()
        self.rows = rows; self.cols = cols; finishName = finish; builtFor = bounds.size
        log.info("board \(rows, privacy: .public)x\(cols, privacy: .public) tile=\(Int(w), privacy: .public) preview=\(self.isPreview, privacy: .public)")
    }

    private func put(_ grid: Grid, rows: Int, cols: Int, finish: String, wave: Wave, gap: Double) {
        if rows != self.rows || cols != self.cols || finish != finishName || tiles.isEmpty || bounds.size != builtFor {
            build(rows: rows, cols: cols, finish: finish)
        }
        guard let faces else { return }
        let now = CACurrentMediaTime()
        let order = Dictionary(uniqueKeysWithValues: tiles.indices.shuffled().enumerated().map { ($1, $0) })
        for (i, t) in tiles.enumerated() {
            let code = t.r < grid.count && t.c < grid[t.r].count ? grid[t.r][t.c] : 0
            t.target = Flaps.index[code] ?? 0
            t.nextAt = now + wave.start(r: t.r, c: t.c, cols: cols, gap: gap, order: order[i] ?? 0)
        }
        // The preview in System Settings shows the board as it is, without turning.
        if isPreview {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            tiles.forEach { $0.settle(faces) }
            CATransaction.commit()
            moving = false
        } else {
            moving = true
        }
    }

    /// Each flap that's due turns one step; the host calls this 30 times a second.
    private func tick() {
        guard moving, let faces else { return }
        let now = CACurrentMediaTime()
        var active = false
        for t in tiles {
            if t.cur == t.target {
                // Landed: tidy up once its last flap has come down.
                if t.nextAt > 0 {
                    if now >= t.nextAt { t.rest(faces); t.nextAt = 0 } else { active = true }
                }
                continue
            }
            active = true
            guard now >= t.nextAt else { continue }
            let d = stepSeconds * t.jitter
            t.step(faces, d)
            t.nextAt = t.cur == t.target ? now + d : max(t.nextAt + d, now + d * 0.5)
        }
        moving = active
    }

    // MARK: burn-in care

    private func drift() {
        lastDrift = Date()
        driftStep += 1
        let room = max(8, (min(bounds.width, bounds.height) * 0.05).rounded())
        let x = (sin(driftStep * 0.61) * room).rounded(), y = (cos(driftStep * 0.37) * room * 0.7).rounded()
        let to = CGPoint(x: bounds.width / 2 + x, y: bounds.height / 2 + y)
        if !isPreview {
            let a = CABasicAnimation(keyPath: "position")
            a.fromValue = stage.position
            a.toValue = to
            a.duration = 6
            a.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            stage.add(a, forKey: "drift")
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        stage.position = isPreview ? CGPoint(x: bounds.width / 2, y: bounds.height / 2) : to
        CATransaction.commit()
    }

    private static let rainbow = [63, 64, 65, 66, 67, 68]
    private static let patterns: [(Int, Int, Int, Int) -> Int] = [
        { r, c, _, _ in rainbow[(r + c) % 6] },
        { _, c, _, _ in rainbow[(c / 2) % 6] },
        { r, c, _, _ in rainbow[(r + 6 - (c % 6)) % 6] },
        { r, c, _, _ in (r + c) % 2 == 1 ? 69 : rainbow[r % 6] },
        { r, c, rows, cols in rainbow[Int((abs(Double(c) - Double(cols - 1) / 2) + abs(Double(r) - Double(rows - 1) / 2)) / 1.5) % 6] },
    ]
    private static let waves: [Wave] = [.diagonal, .column, .edgesToCenter, .reverseColumn, .row]

    /// Every flap turns over: a wave of colour, held a moment, then on round the drums back to
    /// what the board shows now (from colour, every flap passes every letter and number).
    private func wipe() {
        wiping = true
        lastWipe = Date()
        let b = current(), p = Self.patterns[waveIndex % Self.patterns.count], wave = Self.waves[waveIndex % Self.waves.count]
        waveIndex += 1
        let g = (0..<b.rows).map { r in (0..<b.cols).map { c in p(r, c, b.rows, b.cols) } }
        log.info("wave")
        put(g, rows: b.rows, cols: b.cols, finish: b.finish, wave: wave, gap: 0.07)
        DispatchQueue.main.asyncAfter(deadline: .now() + 3.5) { [weak self] in
            guard let self else { return }
            self.wiping = false
            let now = self.current()
            self.shown = "\(now.key)|\(now.grid)|\(now.rows)x\(now.cols)|\(now.finish)"
            self.put(now.grid, rows: now.rows, cols: now.cols, finish: now.finish, wave: wave, gap: 0.05)
        }
    }
}
