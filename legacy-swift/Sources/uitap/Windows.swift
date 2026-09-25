import AppKit
import CoreGraphics
import Foundation

struct WinInfo {
    let id: Int
    let pid: Int
    let app: String
    let title: String
    /// 全局点坐标，左上角为原点。
    let bounds: CGRect
    let layer: Int
    let onscreen: Bool
    /// CGWindowList 给出的前后叠放序号，0 为最前。
    let z: Int
}

func listWindows(includeOffscreen: Bool) -> [WinInfo] {
    let options: CGWindowListOption = includeOffscreen
        ? [.optionAll]
        : [.optionOnScreenOnly, .excludeDesktopElements]
    guard let raw = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { return [] }

    return raw.enumerated().compactMap { position, dict -> WinInfo? in
        guard let number = dict[kCGWindowNumber as String] as? Int,
              let pid = dict[kCGWindowOwnerPID as String] as? Int,
              let boundsDict = dict[kCGWindowBounds as String] as? [String: Any],
              let bounds = CGRect(dictionaryRepresentation: boundsDict as CFDictionary),
              bounds.width >= 2, bounds.height >= 2
        else { return nil }

        return WinInfo(id: number,
                       pid: pid,
                       app: (dict[kCGWindowOwnerName as String] as? String) ?? "",
                       title: (dict[kCGWindowName as String] as? String) ?? "",
                       bounds: bounds,
                       layer: (dict[kCGWindowLayer as String] as? Int) ?? 0,
                       onscreen: (dict[kCGWindowIsOnscreen as String] as? Bool) ?? false,
                       z: position)
    }
}

func findWindow(id: Int) -> WinInfo? {
    listWindows(includeOffscreen: true).first { $0.id == id }
}

func frontmostPID() -> Int {
    Int(NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0)
}

/// 标题截断，避免长标题吃掉 token。
func clipTitle(_ s: String, _ limit: Int = 80) -> String {
    s.count <= limit ? s : String(s.prefix(limit)) + "…"
}

func windowJSON(_ w: WinInfo, frontPID: Int, index: Int) -> [String: Any] {
    var obj: [String: Any] = [
        "id": w.id,
        "pid": w.pid,
        "app": w.app,
        "bounds": rectJSON(w.bounds),
        "layer": w.layer,
        "index": index,
        "z": w.z,
    ]
    if !w.title.isEmpty { obj["title"] = clipTitle(w.title) }
    if !w.onscreen { obj["onscreen"] = false }
    if w.pid == frontPID { obj["front"] = true }
    return obj
}
