// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "AgentArkCompanion",
    platforms: [.iOS(.v16), .macOS(.v13)],
    products: [
        .library(name: "AgentArkCompanionKit", targets: ["AgentArkCompanionKit"])
    ],
    targets: [
        .target(name: "AgentArkCompanionKit")
    ]
)
