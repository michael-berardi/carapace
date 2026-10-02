// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "carapace",
    platforms: [.macOS(.v13), .iOS(.v16)],
    products: [
        // Generic store, wire types and the backend protocols. No Rust symbols needed.
        .library(name: "CarapaceKit", targets: ["CarapaceKit"]),
        // The C ABI binding. Link it together with your core's xcframework.
        .library(name: "CarapaceFFI", targets: ["CarapaceFFI"]),
    ],
    targets: [
        .target(name: "CarapaceKit"),
        .target(name: "CCarapace"),
        .target(name: "CarapaceFFI", dependencies: ["CarapaceKit", "CCarapace"]),
        .testTarget(name: "CarapaceKitTests", dependencies: ["CarapaceKit"]),
    ]
)
