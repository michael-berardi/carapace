// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "CounterApp",
    platforms: [.macOS(.v14)],
    dependencies: [.package(path: "../../..")],
    targets: [
        // Built by `./build.sh` (cargo carapace build apple).
        .binaryTarget(name: "CounterCore", path: "Core/CounterCore.xcframework"),
        .executableTarget(
            name: "CounterApp",
            dependencies: [
                .product(name: "CarapaceKit", package: "carapace"),
                .product(name: "CarapaceFFI", package: "carapace"),
                "CounterCore",
            ]
        ),
        .testTarget(name: "CounterAppTests", dependencies: ["CounterApp"]),
    ]
)
