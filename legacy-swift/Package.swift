// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "uitap",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(name: "uitap", path: "Sources/uitap")
    ]
)
