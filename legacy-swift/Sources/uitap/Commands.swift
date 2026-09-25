import AppKit
import CoreGraphics
import Foundation

// MARK: - 观测目标

/// `--window` / `--region` / `--display` 三选一，都不给则主显示器。
private func observeTarget(_ a: Args, tag: String) -> (window: Int?, region: CGRect?, display: Int?) {
    let window = a.str("window").flatMap { Int($0) }
    let region = a.rect("region")
    let display = a.has("display") ? a.int("display", default: 0) : nil
    return (window, region, display)
}

private func takeShot(_ a: Args, tag: String) -> ShotInfo {
    let target = observeTarget(a, tag: tag)
    let path = a.str("path") ?? defaultShotPath(tag)
    let maxPx = a.has("maxPx") ? a.int("maxPx", default: 0) : nil
    return capture(window: target.window,
                   region: target.region,
                   displayIndex: target.display,
                   outPath: path,
                   maxPx: maxPx)
}

// MARK: - 诊断

func cmdDoctor(_ a: Args) -> Never {
    let permissions = permissionStatus()
    var hints: [String] = []
    if !permissions.screenRecording {
        hints.append("屏幕录制未授权：系统设置 → 隐私与安全性 → 屏幕录制，勾选承载本工具的进程")
    }
    if !permissions.accessibility {
        hints.append("辅助功能未授权：系统设置 → 隐私与安全性 → 辅助功能，勾选同一进程")
    }
    var obj: [String: Any] = [
        "screenRecording": permissions.screenRecording,
        "accessibility": permissions.accessibility,
        "screencapture": FileManager.default.isExecutableFile(atPath: screencapturePath),
        "shotDirectory": shotDirectory(),
    ]
    if !hints.isEmpty { obj["hints"] = hints }
    emit(obj)
}

// MARK: - 观察

func cmdScreens(_ a: Args) -> Never {
    let displays = activeDisplays()
    let list: [[String: Any]] = displays.enumerated().map { index, d in
        var obj: [String: Any] = [
            "index": index,
            "id": Int(d.id),
            "bounds": rectJSON(d.bounds),
            "pixels": ["w": d.pixelWidth, "h": d.pixelHeight],
            "scale": num(d.scale),
        ]
        if d.isMain { obj["main"] = true }
        return obj
    }
    var obj: [String: Any] = ["displays": list]
    if let main = displays.firstIndex(where: { $0.isMain }) { obj["mainIndex"] = main }
    emit(obj)
}

func cmdWindows(_ a: Args) -> Never {
    var windows = listWindows(includeOffscreen: a.flag("all"))
    if let app = a.str("app") {
        let needle = app.lowercased()
        windows = windows.filter { $0.app.lowercased().contains(needle) }
    }
    if let title = a.str("title") {
        let needle = title.lowercased()
        windows = windows.filter { $0.title.lowercased().contains(needle) }
    }
    let layer = a.has("layer") ? a.int("layer", default: 0) : nil
    if let layer { windows = windows.filter { $0.layer == layer } }
    let minWidth = a.double("minWidth", default: 0)
    let minHeight = a.double("minHeight", default: 0)
    if minWidth > 0 { windows = windows.filter { $0.bounds.width >= minWidth } }
    if minHeight > 0 { windows = windows.filter { $0.bounds.height >= minHeight } }
    if a.flag("frontOnly") {
        let pid = frontmostPID()
        windows = windows.filter { $0.pid == pid }
    }

    // 大窗口优先（通常是应用主窗口）；同尺寸时按前后叠放顺序，最前的在前。
    windows.sort { lhs, rhs in
        let leftArea = lhs.bounds.width * lhs.bounds.height
        let rightArea = rhs.bounds.width * rhs.bounds.height
        if leftArea != rightArea { return leftArea > rightArea }
        return lhs.z < rhs.z
    }
    if let limit = a.str("limit").flatMap(Int.init) { windows = Array(windows.prefix(max(0, limit))) }

    let front = frontmostPID()
    let list = windows.enumerated().map { windowJSON($0.element, frontPID: front, index: $0.offset) }
    emit(["count": list.count, "windows": list])
}

func cmdShot(_ a: Args) -> Never {
    let info = takeShot(a, tag: "shot")
    emit(shotJSON(info))
}

func cmdCrop(_ a: Args) -> Never {
    guard let input = a.str("in") else { fail("--in is required") }
    guard FileManager.default.fileExists(atPath: input) else { fail("no such file: \(input)") }
    let out = a.str("out") ?? input.replacingOccurrences(of: ".png", with: "") + "-crop.png"
    let maxPx = a.has("maxPx") ? a.int("maxPx", default: 0) : nil

    // 区域按像素给出的直接裁剪；给 `--regionPoints` 时用锚点换算成像素。
    var pixelRegion = a.rect("region")
    if let regionPoints = a.rect("regionPoints") {
        guard let anchor = anchor(forImage: input, args: a) else {
            fail("--regionPoints needs a sidecar or --scale/--origin")
        }
        let origin = anchor.toPixel(regionPoints.origin)
        let corner = anchor.toPixel(CGPoint(x: regionPoints.maxX, y: regionPoints.maxY))
        pixelRegion = CGRect(x: origin.x, y: origin.y, width: corner.x - origin.x, height: corner.y - origin.y)
    }
    guard let size = cropAndScale(input, region: pixelRegion, maxPx: maxPx, outPath: out) else {
        fail("crop failed: \(input)")
    }
    var obj: [String: Any] = ["path": out, "size": ["w": size.0, "h": size.1]]
    if let anchor = anchor(forImage: input, args: a) {
        let cropWidth = Double(pixelRegion?.width ?? CGFloat(size.0))
        let originX = anchor.origin.x + Double(pixelRegion?.minX ?? 0) / anchor.scale
        let originY = anchor.origin.y + Double(pixelRegion?.minY ?? 0) / anchor.scale
        obj["origin"] = pointJSON(CGPoint(x: originX, y: originY))
        obj["scale"] = num(Double(size.0) / max(1, cropWidth) * anchor.scale)
    }
    emit(obj)
}

func cmdPixel(_ a: Args) -> Never {
    guard let path = a.str("path") else { fail("--path is required") }
    guard let image = loadRGBA(path) else { fail("cannot read image: \(path)") }
    let points = a.points("at")
    guard !points.isEmpty else { fail("--at x,y is required") }

    let anchor = anchor(forImage: path, args: a)
    let usePoint = a.str("units") == "point" || (a.str("units") == nil && anchor != nil)
    guard !usePoint || anchor != nil else { fail("--units point needs a sidecar or --scale/--origin") }

    let results: [[String: Any]] = points.map { input in
        let pixel = (usePoint && anchor != nil) ? anchor!.toPixel(input) : input
        let x = Int(pixel.x.rounded())
        let y = Int(pixel.y.rounded())
        guard let color = image.color(x: x, y: y) else {
            return ["x": num(input.x), "y": num(input.y), "error": "out of bounds",
                    "size": ["w": image.width, "h": image.height]]
        }
        var obj: [String: Any] = [
            "x": num(input.x),
            "y": num(input.y),
            "hex": String(format: "#%02X%02X%02X", color.r, color.g, color.b),
            "rgb": [Int(color.r), Int(color.g), Int(color.b)],
        ]
        if usePoint { obj["px"] = [x, y] }
        return obj
    }
    emit(["units": usePoint ? "point" : "pixel", "points": results])
}

func cmdDiff(_ a: Args) -> Never {
    guard let beforePath = a.str("before"), let afterPath = a.str("after") else {
        fail("--before and --after are required")
    }
    guard let before = loadRGBA(beforePath) else { fail("cannot read image: \(beforePath)") }
    guard let after = loadRGBA(afterPath) else { fail("cannot read image: \(afterPath)") }
    guard before.width == after.width, before.height == after.height else {
        fail("size mismatch: \(before.width)x\(before.height) vs \(after.width)x\(after.height)")
    }

    let anchor = anchor(forImage: beforePath, args: a)
    let usePoint = a.str("units") == "point" || (a.str("units") == nil && anchor != nil)
    guard !usePoint || anchor != nil else { fail("--units point needs a sidecar or --scale/--origin") }

    // `--region` 与 `--units` 同一坐标系：给点坐标时先折算成像素再限定比对范围。
    var limitedTo = a.rect("region")
    if usePoint, let region = limitedTo, let anchor {
        let topLeft = anchor.toPixel(region.origin)
        let bottomRight = anchor.toPixel(CGPoint(x: region.maxX, y: region.maxY))
        limitedTo = CGRect(x: topLeft.x, y: topLeft.y,
                           width: bottomRight.x - topLeft.x,
                           height: bottomRight.y - topLeft.y)
    }

    let result = diffImages(before,
                            after,
                            threshold: a.int("threshold", default: 24),
                            cell: a.int("cell", default: 16),
                            minPixels: a.int("minPixels", default: 12),
                            maxRegions: a.int("maxRegions", default: 6),
                            limitedTo: limitedTo)

    let convert: (CGRect) -> CGRect = usePoint && anchor != nil ? anchor!.toPoint : { $0 }

    var obj: [String: Any] = [
        "changed": result.changed,
        "ratio": ratioValue(result.ratio),
        "changedPixels": result.changedPixels,
        "units": usePoint ? "point" : "pixel",
        "size": ["w": result.width, "h": result.height],
    ]
    if let bounds = result.bounds { obj["bounds"] = rectJSON(convert(bounds)) }
    obj["regions"] = result.regions.map { region -> [String: Any] in
        var item = rectJSON(convert(region.rect))
        item["pixels"] = region.pixels
        return item
    }
    emit(obj)
}

// MARK: - 等待

struct StableOutcome {
    let stable: Bool
    let samples: Int
    let elapsedMs: Int
    let lastRatio: Double
    let lastPath: String
}

/// 反复截图，直到连续若干帧的变化比例低于阈值，或超时。
private func waitForStable(_ a: Args,
                           window: Int?,
                           region: CGRect?,
                           display: Int?,
                           seedPath: String?) -> StableOutcome {
    let interval = max(40, a.int("interval", default: 120))
    let timeout = max(200, a.int("timeout", default: 4000))
    let threshold = a.double("threshold", default: 0.0006)
    let needStable = max(1, a.int("stableSamples", default: 2))
    let seed = seedPath ?? capture(window: window, region: region, displayIndex: display,
                                   outPath: defaultShotPath("wait"), maxPx: nil).path

    let frameA = "\(shotDirectory())/uitap-frame-a.png"
    let frameB = "\(shotDirectory())/uitap-frame-b.png"
    var previous = seed
    var previousIsA = true
    let started = Date()
    var stableCount = 0
    var attempts = 0
    var lastRatio = 1.0

    while Int(Date().timeIntervalSince(started) * 1000) < timeout {
        let next = previousIsA ? frameB : frameA
        capture(window: window, region: region, displayIndex: display, outPath: next, maxPx: nil)
        attempts += 1
        if let lhs = loadRGBA(previous), let rhs = loadRGBA(next) {
            lastRatio = diffImages(lhs, rhs,
                                   threshold: a.int("threshold", default: 24),
                                   cell: a.int("cell", default: 16),
                                   minPixels: 0,
                                   maxRegions: 0,
                                   limitedTo: nil).ratio
            stableCount = lastRatio <= threshold ? stableCount + 1 : 0
            previous = next
            previousIsA = !previousIsA
        }
        if stableCount >= needStable {
            return StableOutcome(stable: true,
                                 samples: attempts,
                                 elapsedMs: Int(Date().timeIntervalSince(started) * 1000),
                                 lastRatio: lastRatio,
                                 lastPath: previous)
        }
        sleepMs(interval)
    }
    return StableOutcome(stable: false,
                         samples: attempts,
                         elapsedMs: Int(Date().timeIntervalSince(started) * 1000),
                         lastRatio: lastRatio,
                         lastPath: previous)
}

func cmdWaitStable(_ a: Args) -> Never {
    let target = observeTarget(a, tag: "wait")
    let outcome = waitForStable(a,
                                window: target.window,
                                region: target.region,
                                display: target.display,
                                seedPath: nil)
    emit([
        "stable": outcome.stable,
        "samples": outcome.samples,
        "elapsedMs": outcome.elapsedMs,
        "lastRatio": ratioValue(outcome.lastRatio),
    ])
}

// MARK: - 输入

func cmdClick(_ a: Args) -> Never {
    guard let point = a.point("at") else { fail("--at x,y is required") }
    let button = MouseButton(rawValue: a.str("button") ?? "left") ?? .left
    clickMouse(at: point,
               button: button,
               count: a.int("count", default: 1),
               holdMs: a.int("hold", default: 40))
    emit(["ok": true, "at": pointJSON(point), "button": button.rawValue, "count": a.int("count", default: 1)])
}

func cmdMove(_ a: Args) -> Never {
    guard let point = a.point("to") else { fail("--to x,y is required") }
    moveMouse(to: point)
    emit(["ok": true, "at": pointJSON(point)])
}

func cmdDrag(_ a: Args) -> Never {
    guard let start = a.point("from"), let end = a.point("to") else { fail("--from x,y and --to x,y are required") }
    let button = MouseButton(rawValue: a.str("button") ?? "left") ?? .left
    dragMouse(from: start, to: end,
              button: button,
              durationMs: a.int("duration", default: 300),
              steps: a.int("steps", default: 20))
    emit(["ok": true, "from": pointJSON(start), "to": pointJSON(end)])
}

func cmdScroll(_ a: Args) -> Never {
    let dy = a.int("dy", default: 0)
    let dx = a.int("dx", default: 0)
    guard dy != 0 || dx != 0 else { fail("--dy or --dx is required") }
    scrollWheel(at: a.point("at"), dy: dy, dx: dx)
    emit(["ok": true, "dx": dx, "dy": dy])
}

func cmdType(_ a: Args) -> Never {
    guard let text = a.str("text") else { fail("--text is required") }
    let delay = a.int("delay", default: 0)
    typeText(text, delayMs: delay)
    emit(["ok": true, "length": text.count, "delayMs": delay])
}

func cmdKey(_ a: Args) -> Never {
    guard let combo = a.str("combo") else { fail("--combo is required") }
    guard let parsed = parseCombo(combo) else { fail("unrecognized combo: \(combo)") }
    let repeatCount = max(1, a.int("repeat", default: 1))
    for index in 0..<repeatCount {
        pressCombo(parsed.keyCode, flags: parsed.flags)
        if index < repeatCount - 1 { sleepMs(40) }
    }
    emit(["ok": true, "combo": combo, "keyCode": Int(parsed.keyCode), "repeat": repeatCount])
}

func cmdActivate(_ a: Args) -> Never {
    var pid: pid_t?
    if let raw = a.str("pid"), let value = Int32(raw) { pid = value }
    if pid == nil, let windowId = a.str("window").flatMap({ Int($0) }), let win = findWindow(id: windowId) {
        pid = Int32(win.pid)
    }
    if pid == nil, let name = a.str("app") {
        let needle = name.lowercased()
        let match = NSWorkspace.shared.runningApplications.first {
            ($0.localizedName ?? "").lowercased() == needle
                || ($0.bundleIdentifier ?? "").lowercased() == needle
        } ?? NSWorkspace.shared.runningApplications.first {
            ($0.localizedName ?? "").lowercased().contains(needle)
        }
        pid = match?.processIdentifier
    }
    guard let target = pid, let app = NSRunningApplication(processIdentifier: target) else {
        fail("application not found")
    }
    if #available(macOS 14.0, *) {
        app.activate()
    } else {
        app.activate(options: [.activateAllWindows])
    }
    sleepMs(a.int("settle", default: 220))
    emit([
        "ok": true,
        "app": app.localizedName ?? "",
        "pid": Int(app.processIdentifier),
        "frontmost": frontmostPID() == Int(app.processIdentifier),
    ])
}

func cmdFrontmost(_ a: Args) -> Never {
    let app = NSWorkspace.shared.frontmostApplication
    emit([
        "app": app?.localizedName ?? "",
        "pid": Int(app?.processIdentifier ?? 0),
        "bundleId": app?.bundleIdentifier ?? "",
    ])
}

// MARK: - 组合动作

/// 点击 → 等画面稳定 → 与点击前比对。一次调用替代 截图/等待/比对 三步。
func cmdTap(_ a: Args) -> Never {
    guard let point = a.point("at") else { fail("--at x,y is required") }
    let target = observeTarget(a, tag: "tap")
    let button = MouseButton(rawValue: a.str("button") ?? "left") ?? .left
    let count = a.int("count", default: 1)
    let keep = a.flag("keep")

    let before = capture(window: target.window, region: target.region, displayIndex: target.display,
                         outPath: defaultShotPath("tap-before"), maxPx: nil)

    clickMouse(at: point, button: button, count: count, holdMs: a.int("hold", default: 40))
    sleepMs(a.int("settle", default: 120))

    let outcome = waitForStable(a,
                                window: target.window,
                                region: target.region,
                                display: target.display,
                                seedPath: before.path)

    guard let base = loadRGBA(before.path), let final = loadRGBA(outcome.lastPath), base.width == final.width else {
        fail("capture failed during tap")
    }
    let result = diffImages(base,
                            final,
                            threshold: a.int("threshold", default: 24),
                            cell: a.int("cell", default: 16),
                            minPixels: a.int("minPixels", default: 12),
                            maxRegions: a.int("maxRegions", default: 6),
                            limitedTo: nil)

    let anchor = Anchor(origin: before.origin, scale: before.scale)
    let convert: (CGRect) -> CGRect = a.str("units") == "pixel" ? { $0 } : anchor.toPoint

    var obj: [String: Any] = [
        "ok": true,
        "at": pointJSON(point),
        "stable": outcome.stable,
        "elapsedMs": outcome.elapsedMs,
        "changed": result.changed,
        "ratio": ratioValue(result.ratio),
        "units": a.str("units") == "pixel" ? "pixel" : "point",
    ]
    if let bounds = result.bounds { obj["bounds"] = rectJSON(convert(bounds)) }
    obj["regions"] = result.regions.map { region -> [String: Any] in
        var item = rectJSON(convert(region.rect))
        item["pixels"] = region.pixels
        return item
    }

    if keep {
        let afterPath = before.path.replacingOccurrences(of: "tap-before", with: "tap-after")
        try? FileManager.default.copyItem(atPath: outcome.lastPath, toPath: afterPath)
        obj["before"] = before.path
        obj["after"] = afterPath
    } else {
        try? FileManager.default.removeItem(atPath: before.path)
        try? FileManager.default.removeItem(atPath: before.path + ".json")
    }
    try? FileManager.default.removeItem(atPath: "\(shotDirectory())/uitap-frame-a.png")
    try? FileManager.default.removeItem(atPath: "\(shotDirectory())/uitap-frame-b.png")
    emit(obj)
}
