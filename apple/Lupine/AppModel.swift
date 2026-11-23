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

    enum DriveState: Equatable {
        case none
        case opening
        case syncing
        case synced(failedRelays: Int)
        case failed(String)
    }

    let coreVersion = LupineCore.coreVersion()
    private let logger = Logger(subsystem: "ovh.uid.lupine", category: "app")
    private let domain = NSFileProviderDomain(
        identifier: NSFileProviderDomainIdentifier(SharedContainer.domainIdentifier),
        displayName: "Lupine"
    )
    private(set) var domainState = DomainState.registering
    private(set) var driveState = DriveState.none
    private(set) var selfTestResult: String?
    private var sync: DriveSync?

    var canOpenDrive: Bool {
        switch driveState {
        case .none, .failed: sync == nil
        default: false
        }
    }

    init() {
        Task {
            await registerDomain()
            await openDrive()
            await runSelfTest()
        }
    }

    func importRecoveryKey(_ recoveryKey: String) async {
        driveState = .opening
        do {
            let path = try SharedContainer.databaseURL().path(percentEncoded: false)
            let imported = try await LupineCore.importRecoveryKey(path: path, recoveryKey: recoveryKey)
            try DriveKeychain.store(secret: imported.secret, drive: imported.id)
        } catch {
            fail("Import failed", error)
            return
        }
        await openDrive()
    }

    private func openDrive() async {
        do {
            let path = try SharedContainer.databaseURL().path(percentEncoded: false)
            guard let local = try openLocalDrive(path: path),
                  let secret = try DriveKeychain.secret(drive: local.info().id)
            else {
                driveState = .none
                return
            }
            driveState = .opening
            let sync = try await openDriveSync(drive: local, secret: secret)
            self.sync = sync
            signalFinder()
            let reporter = SyncReporter(model: self)
            Task {
                do {
                    try await sync.run(observer: reporter)
                } catch {
                    self.sync = nil
                    self.fail("Sync stopped", error)
                }
            }
        } catch {
            fail("Opening the drive failed", error)
        }
    }

    fileprivate func report(_ state: DriveState) {
        driveState = state
    }

    fileprivate func signalFinder() {
        NSFileProviderManager(for: domain)?.signalEnumerator(for: .workingSet) { [logger] error in
            if let error {
                logger.error("Signal failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    private func fail(_ context: String, _ error: Error) {
        driveState = .failed(error.localizedDescription)
        logger.error("\(context, privacy: .public): \(error.localizedDescription, privacy: .public)")
    }

    private func registerDomain() async {
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

/// Called from the core's threads. The main queue keeps reports in order.
private final class SyncReporter: SyncObserver {
    private let model: AppModel

    init(model: AppModel) {
        self.model = model
    }

    func syncing() {
        onMain { $0.report(.syncing) }
    }

    func synced(failed: [String]) {
        onMain { $0.report(.synced(failedRelays: failed.count)) }
    }

    func itemsChanged() {
        onMain { $0.signalFinder() }
    }

    private func onMain(_ update: @escaping @MainActor @Sendable (AppModel) -> Void) {
        let model = model
        DispatchQueue.main.async {
            MainActor.assumeIsolated { update(model) }
        }
    }
}
