import FileProvider
import LupineCore
import Observation
import os

@MainActor
@Observable
final class AppModel {
    enum DomainState: Equatable {
        case registering
        case registered
        case failed(String)
    }

    let coreVersion = LupineCore.coreVersion()
    private let logger = Logger(subsystem: "ovh.uid.lupine", category: "app")
    private(set) var domainState = DomainState.registering
    private(set) var selfTestResult: String?

    init() {
        Task {
            await registerDomain()
            await runSelfTest()
        }
    }

    private func registerDomain() async {
        let domain = NSFileProviderDomain(
            identifier: NSFileProviderDomainIdentifier("drive"),
            displayName: "Lupine"
        )
        do {
            try await NSFileProviderManager.add(domain)
            domainState = .registered
        } catch {
            domainState = .failed(error.localizedDescription)
            logger.error("Domain registration failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    private func runSelfTest() async {
        let size: UInt64 = 64 << 20
        do {
            let report = try await selfTest(size: size)
            selfTestResult = "\(report.bytes >> 20) MiB, \(report.blobs) blobs, \(report.millis) ms"
            logger.info("Core self-test passed: \(self.selfTestResult ?? "", privacy: .public)")
        } catch {
            selfTestResult = "Failed: \(error.localizedDescription)"
            logger.error("Core self-test failed: \(error.localizedDescription, privacy: .public)")
        }
    }
}
