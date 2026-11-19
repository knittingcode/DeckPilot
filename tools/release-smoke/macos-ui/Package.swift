// swift-tools-version:5.9
// The macOS accessibility driver of the release smoke test (see
// Sources/macos-ui/main.swift). A package rather than a loose file so that
// CodeQL's Swift autobuild can build, and therefore scan, it:
//   swift build -c release --package-path tools/release-smoke/macos-ui
import PackageDescription

let package = Package(
    name: "macos-ui",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(name: "macos-ui", path: "Sources/macos-ui")
    ]
)
