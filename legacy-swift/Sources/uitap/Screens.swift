import AppKit
import CoreGraphics
import Foundation

struct DisplayInfo {
    let id: CGDirectDisplayID
    /// 全局点坐标，左上角为原点。与 CGEvent 的光标坐标同一坐标系。
    let bounds: CGRect
    let pixelWidth: Int
    let pixelHeight: Int

    var scale: Double { bounds.width > 0 ? Double(pixelWidth) / Double(bounds.width) : 1 }
    var isMain: Bool { CGMainDisplayID() == id }
}

func activeDisplays() -> [DisplayInfo] {
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else { return [] }
    var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
    guard CGGetActiveDisplayList(count, &ids, &count) == .success else { return [] }
    return ids.prefix(Int(count)).map { id in
        DisplayInfo(id: id,
                    bounds: CGDisplayBounds(id),
                    pixelWidth: CGDisplayPixelsWide(id),
                    pixelHeight: CGDisplayPixelsHigh(id))
    }
}

func mainDisplay() -> DisplayInfo? {
    activeDisplays().first { $0.isMain } ?? activeDisplays().first
}

/// 屏幕录制与辅助功能两项授权状态。
func permissionStatus() -> (screenRecording: Bool, accessibility: Bool) {
    (CGPreflightScreenCaptureAccess(), AXIsProcessTrusted())
}
