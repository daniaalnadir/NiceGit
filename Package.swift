// swift-tools-version: 6.3
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "NiceGit",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .executable(name: "NiceGit", targets: ["NiceGit"]),
        .library(name: "NiceGitCore", targets: ["NiceGitCore"])
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", exact: "1.20.0")
    ],
    targets: [
        .target(
            name: "NiceGitCore"
        ),
        .executableTarget(
            name: "NiceGit",
            dependencies: ["NiceGitCore", .product(name: "SwiftTerm", package: "SwiftTerm")]
        ),
        .testTarget(
            name: "NiceGitCoreTests",
            dependencies: ["NiceGitCore"]
        ),
        .testTarget(name: "NiceGitAppTests", dependencies: ["NiceGit", "NiceGitCore"]),
    ],
    swiftLanguageModes: [.v6]
)
