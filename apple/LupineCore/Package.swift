// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "LupineCore",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "LupineCore", targets: ["LupineCore"]),
    ],
    targets: [
        .binaryTarget(name: "LupineFFI", path: "LupineFFI.xcframework"),
        .target(
            name: "LupineCore",
            dependencies: ["LupineFFI"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            // The Rust library verifies TLS with the system trust store and reads the system proxy settings.
            linkerSettings: [.linkedFramework("Security"), .linkedFramework("SystemConfiguration")]
        ),
    ]
)
