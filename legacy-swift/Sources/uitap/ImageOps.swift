import CoreGraphics
import Foundation
import ImageIO

/// RGBA8 位图，行 0 对应图像顶部（与 PNG、屏幕点坐标同向）。
struct RGBAImage {
    let width: Int
    let height: Int
    let data: [UInt8]

    func color(x: Int, y: Int) -> (r: UInt8, g: UInt8, b: UInt8, a: UInt8)? {
        guard x >= 0, y >= 0, x < width, y < height else { return nil }
        let i = (y * width + x) * 4
        return (data[i], data[i + 1], data[i + 2], data[i + 3])
    }
}

func imagePixelSize(_ path: String) -> (Int, Int)? {
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
          let props = CGImageSourceCopyPropertiesAtIndex(src, 0, nil) as? [CFString: Any],
          let w = props[kCGImagePropertyPixelWidth] as? Int,
          let h = props[kCGImagePropertyPixelHeight] as? Int
    else { return nil }
    return (w, h)
}

/// 目标位图直接沿用源图的色彩空间。用 DeviceRGB 会让 CoreGraphics 做一次色彩转换，
/// 取到的值就不再是 PNG 里的原始像素（也与 PIL 等按原始值读取的工具对不上）。
private func bitmapSpace(for image: CGImage) -> CGColorSpace {
    if let space = image.colorSpace, space.model == .rgb, space.numberOfComponents == 3 {
        return space
    }
    return CGColorSpaceCreateDeviceRGB()
}

func loadRGBA(_ path: String) -> RGBAImage? {
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
          let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { return nil }

    let width = cg.width
    let height = cg.height
    var buffer = [UInt8](repeating: 0, count: width * height * 4)
    let space = bitmapSpace(for: cg)

    let ok = buffer.withUnsafeMutableBytes { raw -> Bool in
        guard let ctx = CGContext(data: raw.baseAddress,
                                  width: width,
                                  height: height,
                                  bitsPerComponent: 8,
                                  bytesPerRow: width * 4,
                                  space: space,
                                  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return false }
        ctx.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
        return true
    }
    guard ok else { return nil }
    return RGBAImage(width: width, height: height, data: buffer)
}

struct DiffRegion {
    let rect: CGRect
    let pixels: Int
}

struct DiffResult {
    let width: Int
    let height: Int
    let changedPixels: Int
    let sampledPixels: Int
    /// 像素坐标，左上原点。
    let bounds: CGRect?
    let regions: [DiffRegion]

    var ratio: Double { sampledPixels > 0 ? Double(changedPixels) / Double(sampledPixels) : 0 }
    var changed: Bool { changedPixels > 0 }
}

/// 逐像素比对；用单元格聚合 + 连通填充归并成若干变化区域，区域边界取实际变化像素的极值。
func diffImages(_ a: RGBAImage,
                _ b: RGBAImage,
                threshold: Int,
                cell: Int,
                minPixels: Int,
                maxRegions: Int,
                limitedTo region: CGRect?) -> DiffResult {
    precondition(a.width == b.width && a.height == b.height, "image size mismatch")

    let width = a.width
    let height = a.height
    let scan = (region ?? CGRect(x: 0, y: 0, width: width, height: height)).intersection(
        CGRect(x: 0, y: 0, width: width, height: height)).integral

    let x0 = max(0, Int(scan.minX))
    let y0 = max(0, Int(scan.minY))
    let x1 = min(width, Int(scan.maxX))
    let y1 = min(height, Int(scan.maxY))
    guard x1 > x0, y1 > y0 else {
        return DiffResult(width: width, height: height, changedPixels: 0, sampledPixels: 0, bounds: nil, regions: [])
    }

    let cellSize = max(1, cell)
    let cols = (x1 - x0 + cellSize - 1) / cellSize
    let rows = (y1 - y0 + cellSize - 1) / cellSize

    var cellHit = [Bool](repeating: false, count: cols * rows)
    var cellMinX = [Int](repeating: Int.max, count: cols * rows)
    var cellMinY = [Int](repeating: Int.max, count: cols * rows)
    var cellMaxX = [Int](repeating: Int.min, count: cols * rows)
    var cellMaxY = [Int](repeating: Int.min, count: cols * rows)
    var cellCount = [Int](repeating: 0, count: cols * rows)

    var changedPixels = 0
    var minX = Int.max, minY = Int.max, maxX = Int.min, maxY = Int.min

    a.data.withUnsafeBufferPointer { pa in
        b.data.withUnsafeBufferPointer { pb in
            for y in y0..<y1 {
                let rowBase = y * width * 4
                let gy = (y - y0) / cellSize
                for x in x0..<x1 {
                    let i = rowBase + x * 4
                    let delta = abs(Int(pa[i]) - Int(pb[i]))
                        + abs(Int(pa[i + 1]) - Int(pb[i + 1]))
                        + abs(Int(pa[i + 2]) - Int(pb[i + 2]))
                    guard delta > threshold else { continue }
                    changedPixels += 1
                    if x < minX { minX = x }
                    if y < minY { minY = y }
                    if x > maxX { maxX = x }
                    if y > maxY { maxY = y }
                    let ci = gy * cols + (x - x0) / cellSize
                    cellHit[ci] = true
                    cellCount[ci] += 1
                    if x < cellMinX[ci] { cellMinX[ci] = x }
                    if y < cellMinY[ci] { cellMinY[ci] = y }
                    if x > cellMaxX[ci] { cellMaxX[ci] = x }
                    if y > cellMaxY[ci] { cellMaxY[ci] = y }
                }
            }
        }
    }

    var regions: [DiffRegion] = []
    if changedPixels > 0 {
        var visited = [Bool](repeating: false, count: cols * rows)
        var stack: [Int] = []
        for start in 0..<(cols * rows) where cellHit[start] && !visited[start] {
            visited[start] = true
            stack = [start]
            var rMinX = Int.max, rMinY = Int.max, rMaxX = Int.min, rMaxY = Int.min
            var sum = 0
            while let idx = stack.popLast() {
                if cellMinX[idx] != Int.max {
                    rMinX = min(rMinX, cellMinX[idx])
                    rMinY = min(rMinY, cellMinY[idx])
                    rMaxX = max(rMaxX, cellMaxX[idx])
                    rMaxY = max(rMaxY, cellMaxY[idx])
                }
                sum += cellCount[idx]
                let cy = idx / cols, cx = idx % cols
                for dy in -1...1 {
                    for dx in -1...1 {
                        let nx = cx + dx, ny = cy + dy
                        guard nx >= 0, ny >= 0, nx < cols, ny < rows else { continue }
                        let ni = ny * cols + nx
                        if cellHit[ni] && !visited[ni] {
                            visited[ni] = true
                            stack.append(ni)
                        }
                    }
                }
            }
            guard rMinX != Int.max else { continue }
            regions.append(DiffRegion(rect: CGRect(x: rMinX,
                                                   y: rMinY,
                                                   width: rMaxX - rMinX + 1,
                                                   height: rMaxY - rMinY + 1),
                                      pixels: sum))
        }
        regions.sort { $0.pixels > $1.pixels }
        if minPixels > 0 { regions = regions.filter { $0.pixels >= minPixels } }
        if maxRegions > 0 && regions.count > maxRegions { regions = Array(regions.prefix(maxRegions)) }
    }

    let bounds: CGRect? = changedPixels > 0
        ? CGRect(x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1)
        : nil

    return DiffResult(width: width,
                      height: height,
                      changedPixels: changedPixels,
                      sampledPixels: (x1 - x0) * (y1 - y0),
                      bounds: bounds,
                      regions: regions)
}

/// 裁剪并按最长边缩放，写出 PNG。返回最终像素尺寸。
func cropAndScale(_ inPath: String, region: CGRect?, maxPx: Int?, outPath: String) -> (Int, Int)? {
    guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: inPath) as CFURL, nil),
          let cg = CGImageSourceCreateImageAtIndex(src, 0, nil)
    else { return nil }

    let full = CGRect(x: 0, y: 0, width: cg.width, height: cg.height)
    let crop = (region ?? full).intersection(full).integral
    guard crop.width >= 1, crop.height >= 1, let sub = cg.cropping(to: crop) else { return nil }

    var targetW = Int(crop.width)
    var targetH = Int(crop.height)
    if let limit = maxPx, limit > 0, max(targetW, targetH) > limit {
        let k = Double(limit) / Double(max(targetW, targetH))
        targetW = max(1, Int((Double(targetW) * k).rounded()))
        targetH = max(1, Int((Double(targetH) * k).rounded()))
    }

    let space = bitmapSpace(for: cg)
    guard let ctx = CGContext(data: nil,
                              width: targetW,
                              height: targetH,
                              bitsPerComponent: 8,
                              bytesPerRow: 0,
                              space: space,
                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return nil }
    ctx.interpolationQuality = .high
    ctx.draw(sub, in: CGRect(x: 0, y: 0, width: targetW, height: targetH))
    guard let out = ctx.makeImage() else { return nil }

    let url = URL(fileURLWithPath: outPath)
    guard let dest = CGImageDestinationCreateWithURL(url as CFURL, "public.png" as CFString, 1, nil) else { return nil }
    CGImageDestinationAddImage(dest, out, nil)
    guard CGImageDestinationFinalize(dest) else { return nil }
    return (targetW, targetH)
}
