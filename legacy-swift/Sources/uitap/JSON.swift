import CoreGraphics
import Foundation

/// 输出紧凑 JSON 并结束进程。退出码 0。
func emit(_ obj: Any) -> Never {
    writeJSON(obj)
    exit(0)
}

/// 输出 `{"error": ...}` 并结束进程。退出码 1。
func fail(_ message: String) -> Never {
    writeJSON(["error": message])
    exit(1)
}

private func writeJSON(_ obj: Any) {
    let data: Data
    if let encoded = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys, .withoutEscapingSlashes]) {
        data = encoded
    } else {
        data = Data(#"{"error":"encode failed"}"#.utf8)
    }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

/// 整数化输出：整数值不显示小数点，其余保留一位，压低体积。
/// 小数走 NSDecimalNumber，避免 Double 打印出 `0.035000000000000003` 这类尾巴。
func num(_ d: Double) -> Any {
    guard d.isFinite else { return 0 }
    let rounded = (d * 10).rounded() / 10
    if rounded == rounded.rounded(), abs(rounded) < 1e9 { return Int(rounded) }
    return NSDecimalNumber(string: String(format: "%.1f", d))
}

/// 比例等小数值：固定保留位，不做整数化。
func ratioValue(_ d: Double, _ places: Int = 4) -> NSDecimalNumber {
    guard d.isFinite else { return NSDecimalNumber.zero }
    return NSDecimalNumber(string: String(format: "%.\(places)f", d))
}

func pointJSON(_ p: CGPoint) -> [String: Any] {
    ["x": num(p.x), "y": num(p.y)]
}

func rectJSON(_ r: CGRect) -> [String: Any] {
    ["x": num(r.minX), "y": num(r.minY), "w": num(r.width), "h": num(r.height)]
}

func sizeJSON(_ s: CGSize) -> [String: Any] {
    ["w": num(s.width), "h": num(s.height)]
}
