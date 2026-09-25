import CoreGraphics
import Foundation

/// 命令行解析：`--key value`、`--key=value`、裸开关 `--key`；重复的 key 累积成数组。
/// 以 `-` 开头的数值不会被当作 key，`--origin -100,20` 可用。
struct Args {
    private var values: [String: [String]] = [:]
    private(set) var positionals: [String] = []

    init(_ raw: [String]) {
        var i = 0
        while i < raw.count {
            let token = raw[i]
            guard token.hasPrefix("--") else {
                positionals.append(token)
                i += 1
                continue
            }
            let body = String(token.dropFirst(2))
            if let eq = body.firstIndex(of: "=") {
                let key = String(body[body.startIndex..<eq])
                values[key, default: []].append(String(body[body.index(after: eq)...]))
            } else if i + 1 < raw.count, !raw[i + 1].hasPrefix("--") {
                values[body, default: []].append(raw[i + 1])
                i += 1
            } else {
                values[body, default: []].append("true")
            }
            i += 1
        }
    }

    func all(_ key: String) -> [String] { values[key] ?? [] }
    func has(_ key: String) -> Bool { values[key] != nil }
    func str(_ key: String) -> String? { values[key]?.last }

    func flag(_ key: String, default def: Bool = false) -> Bool {
        guard let v = values[key]?.last else { return def }
        return v != "false" && v != "0"
    }

    func int(_ key: String, default def: Int) -> Int {
        guard let v = values[key]?.last, let n = Int(v) else { return def }
        return n
    }

    func double(_ key: String, default def: Double) -> Double {
        guard let v = values[key]?.last, let n = Double(v) else { return def }
        return n
    }

    /// 接受 `12,34`、`12 34`、`12x34` 三种写法。
    private static func numbers(_ raw: String) -> [Double] {
        raw.split(whereSeparator: { $0 == "," || $0 == " " || $0 == "x" || $0 == "X" })
            .compactMap { Double($0) }
    }

    func point(_ key: String) -> CGPoint? {
        guard let raw = values[key]?.last else { return nil }
        let n = Args.numbers(raw)
        guard n.count >= 2 else { return nil }
        return CGPoint(x: n[0], y: n[1])
    }

    func rect(_ key: String) -> CGRect? {
        guard let raw = values[key]?.last else { return nil }
        let n = Args.numbers(raw)
        guard n.count >= 4 else { return nil }
        return CGRect(x: n[0], y: n[1], width: n[2], height: n[3])
    }

    func points(_ key: String) -> [CGPoint] {
        all(key).compactMap { raw in
            let n = Args.numbers(raw)
            guard n.count >= 2 else { return nil }
            return CGPoint(x: n[0], y: n[1])
        }
    }
}
