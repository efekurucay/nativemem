// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "nativemem",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Nativemem", targets: ["Nativemem"]),
    ],
    targets: [
        .target(name: "NativememKit"),
        .executableTarget(name: "Nativemem", dependencies: ["NativememKit"]),
        .testTarget(
            name: "NativememKitTests",
            dependencies: ["NativememKit"],
            resources: [.copy("Fixtures")]
        ),
    ],
    swiftLanguageModes: [.v6]
)
