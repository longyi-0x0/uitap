import CoreGraphics
import Foundation

/// 截图的点坐标锚点：像素 → 全局点的换算是 `origin + pixel / scale`。
/// `scale` 由最终图像的像素宽除以对应点宽得出，因此缩放过的图依然换算正确。
struct ShotInfo {
    let path: String
    let pixelWidth: Int
    let pixelHeight: Int
    let origin: CGPoint
    let pointWidth: Double
    let pointHeight: Double
    let mode: String
    let windowId: Int?

    var scale: Double { pointWidth > 0 ? Double(pixelWidth) / pointWidth : 1 }

    /// 像素坐标 → 全局点坐标。
    func toPoint(_ pixel: CGRect) -> CGRect {
        let s = scale == 0 ? 1 : scale
        return CGRect(x: origin.x + pixel.minX / s,
                      y: origin.y + pixel.minY / s,
                      width: pixel.width / s,
                      height: pixel.height / s)
    }
}

let screencapturePath = "/usr/sbin/screencapture"

@discardableResult
func runProcess(_ launchPath: String, _ arguments: [String]) -> (status: Int32, stderr: String) {
    let proc = Process()
    proc.executableURL = URL(fileURLWithPath: launchPath)
    proc.arguments = arguments
    proc.standardOutput = FileHandle.nullDevice
    let errorPipe = Pipe()
    proc.standardError = errorPipe
    do { try proc.run() } catch { fail("cannot run \(launchPath): \(error.localizedDescription)") }
    let errorData = errorPipe.fileHandleForReading.readDataToEndOfFile()
    proc.waitUntilExit()
    return (proc.terminationStatus, String(data: errorData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "")
}

func shotDirectory() -> String {
    let dir = "/tmp/uitap"
    try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    return dir
}

/// 目录内文件过多时裁掉较旧的一半，避免临时截图无限堆积。
func pruneShotDirectory(keep limit: Int = 240) {
    let dir = shotDirectory()
    let protected = ["uitap-frame-a.png", "uitap-frame-b.png"]
    guard let names = try? FileManager.default.contentsOfDirectory(atPath: dir), names.count > limit else { return }
    let entries = names
        .filter { !protected.contains($0) }
        .compactMap { name -> (String, Date)? in
        let full = dir + "/" + name
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: full),
              let date = attrs[.creationDate] as? Date else { return (full, .distantPast) }
        return (full, date)
    }.sorted { $0.1 < $1.1 }
    for (path, _) in entries.prefix(entries.count - limit / 2) {
        try? FileManager.default.removeItem(atPath: path)
    }
}

func defaultShotPath(_ tag: String) -> String {
    let ms = Int(Date().timeIntervalSince1970 * 1000)
    return "\(shotDirectory())/\(tag)-\(ms).png"
}

/// 执行一次截图。`window` / `region` / `display` 三选一，都不给则截主显示器。
@discardableResult
func capture(window: Int?, region: CGRect?, displayIndex: Int?, outPath: String, maxPx: Int?) -> ShotInfo {
    let info = captureRaw(window: window, region: region, displayIndex: displayIndex, outPath: outPath, maxPx: maxPx)
    writeSidecar(info)
    return info
}

/// 截图锚点随图落盘，后续 `pixel` / `diff` 不必再传 scale 与 origin。
private func writeSidecar(_ info: ShotInfo) {
    var obj: [String: Any] = [
        "path": info.path,
        "origin": ["x": info.origin.x, "y": info.origin.y],
        "scale": info.scale,
        "size": ["w": info.pixelWidth, "h": info.pixelHeight],
        "mode": info.mode,
    ]
    if let id = info.windowId { obj["window"] = id }
    guard let data = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys]) else { return }
    try? data.write(to: URL(fileURLWithPath: info.path + ".json"))
}

private func captureRaw(window: Int?, region: CGRect?, displayIndex: Int?, outPath: String, maxPx: Int?) -> ShotInfo {
    pruneShotDirectory()

    guard FileManager.default.isExecutableFile(atPath: screencapturePath) else {
        fail("\(screencapturePath) not found")
    }
    let parent = URL(fileURLWithPath: outPath).deletingLastPathComponent().path
    guard FileManager.default.fileExists(atPath: parent) else {
        fail("output directory does not exist: \(parent)")
    }

    var args = ["-x", "-t", "png"]
    var origin = CGPoint.zero
    var pointWidth = 0.0
    var pointHeight = 0.0
    var mode = "screen"
    var resolvedWindow: Int?

    if let id = window {
        guard let win = findWindow(id: id) else { fail("window \(id) not found") }
        args += ["-o", "-l", String(id)]
        origin = win.bounds.origin
        pointWidth = win.bounds.width
        pointHeight = win.bounds.height
        mode = "window"
        resolvedWindow = id
    } else if let rect = region {
        let r = rect.integral
        args += ["-R", "\(Int(r.minX)),\(Int(r.minY)),\(Int(r.width)),\(Int(r.height))"]
        origin = r.origin
        pointWidth = r.width
        pointHeight = r.height
        mode = "region"
    } else {
        let displays = activeDisplays()
        guard !displays.isEmpty else { fail("no active display") }
        let index = displayIndex ?? displays.firstIndex { $0.isMain } ?? 0
        guard index >= 0, index < displays.count else { fail("display index \(index) out of range") }
        let d = displays[index]
        let r = d.bounds.integral
        args += ["-R", "\(Int(r.minX)),\(Int(r.minY)),\(Int(r.width)),\(Int(r.height))"]
        origin = r.origin
        pointWidth = r.width
        pointHeight = r.height
        mode = "screen"
    }

    // 需要缩放时先落原图，再裁剪缩放覆盖，保证像素坐标换算始终基于最终文件。
    if let limit = maxPx {
        let staging = outPath + ".raw.png"
        args.append(staging)
        let staged = runProcess(screencapturePath, args)
        guard staged.status == 0 else {
            fail("screencapture failed (region \(outPath)): \(staged.stderr.isEmpty ? "no stderr" : staged.stderr)")
        }
        guard let final = cropAndScale(staging, region: nil, maxPx: limit, outPath: outPath) else {
            fail("resize failed: \(staging) -> \(outPath)")
        }
        try? FileManager.default.removeItem(atPath: staging)
        return ShotInfo(path: outPath,
                        pixelWidth: final.0,
                        pixelHeight: final.1,
                        origin: origin,
                        pointWidth: pointWidth,
                        pointHeight: pointHeight,
                        mode: mode,
                        windowId: resolvedWindow)
    }

    args.append(outPath)
    let direct = runProcess(screencapturePath, args)
    guard direct.status == 0 else {
        fail("screencapture failed (\(outPath)): \(direct.stderr.isEmpty ? "no stderr" : direct.stderr)")
    }
    guard let size = imagePixelSize(outPath) else { fail("cannot read capture: \(outPath)") }
    return ShotInfo(path: outPath,
                    pixelWidth: size.0,
                    pixelHeight: size.1,
                    origin: origin,
                    pointWidth: pointWidth,
                    pointHeight: pointHeight,
                    mode: mode,
                    windowId: resolvedWindow)
}

func shotJSON(_ s: ShotInfo) -> [String: Any] {
    var obj: [String: Any] = [
        "path": s.path,
        "size": ["w": s.pixelWidth, "h": s.pixelHeight],
        "origin": pointJSON(s.origin),
        "scale": num(s.scale),
        "mode": s.mode,
    ]
    if let id = s.windowId { obj["window"] = id }
    return obj
}

/// 像素坐标与全局点坐标之间的换算依据。
struct Anchor {
    let origin: CGPoint
    let scale: Double

    func toPoint(_ r: CGRect) -> CGRect {
        let s = scale == 0 ? 1 : scale
        return CGRect(x: origin.x + r.minX / s,
                      y: origin.y + r.minY / s,
                      width: r.width / s,
                      height: r.height / s)
    }

    func toPixel(_ p: CGPoint) -> CGPoint {
        CGPoint(x: (p.x - origin.x) * scale, y: (p.y - origin.y) * scale)
    }
}

private func asDouble(_ any: Any?) -> Double? {
    (any as? NSNumber)?.doubleValue
}

/// 读取截图旁的 `.json` 锚点文件。也可用 `--scale` / `--origin` 显式覆盖。
func anchor(forImage path: String, args: Args) -> Anchor? {
    if let scale = args.str("scale").flatMap(Double.init), let origin = args.point("origin"), scale > 0 {
        return Anchor(origin: origin, scale: scale)
    }
    guard let data = FileManager.default.contents(atPath: path + ".json"),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let sideOrigin = obj["origin"] as? [String: Any],
          let x = asDouble(sideOrigin["x"]),
          let y = asDouble(sideOrigin["y"]),
          let scale = asDouble(obj["scale"]),
          scale > 0
    else { return nil }
    return Anchor(origin: CGPoint(x: x, y: y), scale: scale)
}
