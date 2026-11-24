import AppKit
import FileProvider
import LupineCore
import Observation
import ServiceManagement
import os

@MainActor
@Observable
final class AppModel {
    enum DomainState: Equatable {
        case none
        case registering
        case registered
        case failed(String)
    }

    struct Fill: Equatable {
        var done: UInt64
        var total: UInt64
    }

    enum DriveState: Equatable {
        case none
        case opening
        case syncing
        case synced(failedRelays: Int)
        case failed(String)
    }

    let coreVersion = LupineCore.coreVersion()
    let windows = Windows()
    private let logger = Logger(subsystem: "ovh.uid.lupine", category: "app")
    private let domain = NSFileProviderDomain(
        identifier: NSFileProviderDomainIdentifier(SharedContainer.domainIdentifier),
        displayName: "Lupine"
    )
    private(set) var domainState = DomainState.none
    private(set) var driveState = DriveState.none
    private(set) var selfTestResult: String?
    private(set) var folders: SyncedFolders?
    private(set) var info: DriveInfo?
    private(set) var historyDays: UInt32 = 30
    /// Blobs being copied to the drive's servers.
    private(set) var fill: Fill?
    private(set) var opensAtLogin = SMAppService.mainApp.status == .enabled
    private(set) var loginItemNeedsApproval = SMAppService.mainApp.status == .requiresApproval
    private var local: LocalDrive?
    private var sync: DriveSync?
    private var syncing: Task<Void, Never>?
    private var outbox: OutboxSignal.Listener?
    private var keptDescriptor = false
    private var filling: Task<Void, Never>?
    private var fillStop: Cancellation?
    private var lastFill: Date?
    private var collecting: Task<Void, Never>?
    private var collectionStop: Cancellation?
    private var lastCollection: Date?

    var hasDrive: Bool {
        local != nil
    }

    init() {
        Task {
            await openDrive()
            if local == nil {
                showOnboarding()
            }
            await runSelfTest()
        }
    }

    func showOnboarding() {
        guard local == nil else { return }
        let onboarding = Onboarding(app: self)
        windows.show(.onboarding, title: "Lupine", onClose: { onboarding.cancel() }) {
            OnboardingView(onboarding: onboarding)
        }
    }

    func showSettings() {
        refreshLoginItem()
        windows.show(.settings, title: "Lupine Settings") {
            SettingsView(model: self)
        }
    }

    func importRecoveryKey(_ recoveryKey: String) async throws {
        let path = try SharedContainer.databaseURL().path(percentEncoded: false)
        try await setUp(try await LupineCore.importRecoveryKey(path: path, recoveryKey: recoveryKey))
    }

    /// The database is set up for `drive`: keep its secret and open it.
    func setUp(_ drive: ImportedDrive) async throws {
        try DriveKeychain.store(secret: drive.secret, drive: drive.id)
        await openDrive()
    }

    func recoveryKey() -> String? {
        do {
            guard let local, let id = info?.id, let secret = try DriveKeychain.secret(drive: id) else {
                return nil
            }
            return try local.recoveryKey(secret: secret)
        } catch {
            logger.error("Reading the recovery key failed: \(error.localizedDescription, privacy: .public)")
            return nil
        }
    }

    func setHistoryDays(_ days: UInt32) {
        do {
            try local?.setHistoryDays(days: days)
            historyDays = days
        } catch {
            logger.error("Changing the history window failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// Publishes the drive's new lists, then opens the drive again with them.
    func changeLists(relays: [String], servers: [String]) async throws {
        guard let local, let id = info?.id, let secret = try DriveKeychain.secret(drive: id) else {
            return
        }
        let changed = try await LupineCore.changeLists(drive: local, secret: secret, relays: relays, servers: servers)
        if changed.relays || changed.servers {
            await reopenDrive()
        }
    }

    func unsentChanges() -> UInt64 {
        (try? local?.unsent()) ?? 0
    }

    func setOpenAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            logger.error("Changing the login item failed: \(error.localizedDescription, privacy: .public)")
        }
        refreshLoginItem()
    }

    /// Removes the drive from this Mac: Finder's copy of it, the synced
    /// folder pairs, the local database and the drive secret. Files Finder
    /// had not handed over yet are kept aside and shown.
    func signOut() async {
        guard let local else { return }
        let id = try? local.info().id
        await closeDrive()
        info = nil
        do {
            if let kept = try await NSFileProviderManager.remove(domain, mode: .preserveDirtyUserData) {
                NSWorkspace.shared.activateFileViewerSelecting([kept])
            }
        } catch {
            logger.error("Removing the Finder location failed: \(error.localizedDescription, privacy: .public)")
        }
        domainState = .none
        do {
            if let id {
                try DriveKeychain.delete(drive: id)
            }
            try SharedContainer.removeDatabase()
        } catch {
            logger.error("Forgetting the drive failed: \(error.localizedDescription, privacy: .public)")
        }
        windows.close(.settings)
        showOnboarding()
    }

    /// Stops everything that uses the drive, which stays set up.
    private func closeDrive() async {
        fillStop?.cancel()
        collectionStop?.cancel()
        await filling?.value
        await collecting?.value
        sync?.stop()
        await syncing?.value
        await folders?.stop()
        outbox = nil
        sync = nil
        syncing = nil
        folders = nil
        local = nil
        fill = nil
        lastFill = nil
        lastCollection = nil
        keptDescriptor = false
        driveState = .none
    }

    /// The drive's relays or servers changed.
    fileprivate func reopenDrive() async {
        guard local != nil else { return }
        await closeDrive()
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
            self.local = local
            info = try local.info()
            historyDays = try local.historyDays()
            driveState = .opening
            await registerDomain()
            let sync = try await openDriveSync(drive: local, secret: secret)
            self.sync = sync
            let folderSync = try openFolderSync(drive: local, secret: secret, trash: Trasher())
            outbox = OutboxSignal.Listener {
                sync.written()
                folderSync.changed()
            }
            signalFinder()
            let reporter = SyncReporter(model: self)
            syncing = Task {
                do {
                    try await sync.run(observer: reporter)
                } catch {
                    self.sync = nil
                    self.outbox = nil
                    self.fail("Sync stopped", error)
                }
            }
            let folders = SyncedFolders(sync: folderSync, domain: domain) { [weak self] in
                self?.sync?.written()
                self?.signalFinder()
            }
            self.folders = folders
            await folders.start()
        } catch {
            fail("Opening the drive failed", error)
        }
    }

    fileprivate func report(_ state: DriveState) {
        driveState = state
    }

    /// Once per launch, after the relays answered.
    fileprivate func keepDescriptor() {
        guard !keptDescriptor, let sync else { return }
        keptDescriptor = true
        Task { [logger] in
            do {
                _ = try await sync.keepDescriptor()
            } catch {
                logger.error("Republishing the descriptor failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    /// Copies the blobs the drive's new servers lack, once the session synced
    /// every relay and knows every version. A fill cut short or failed tries
    /// again after an hour.
    fileprivate func startFill() {
        guard filling == nil, let local, info?.filling == true,
              lastFill.map({ $0.timeIntervalSinceNow < -3600 }) ?? true,
              let id = info?.id, let secret = try? DriveKeychain.secret(drive: id)
        else { return }
        let stop = Cancellation()
        fillStop = stop
        lastFill = Date()
        fill = Fill(done: 0, total: 0)
        let reporter = FillReporter(model: self)
        filling = Task {
            do {
                let report = try await LupineCore.fillServers(drive: local, secret: secret, observer: reporter, cancellation: stop)
                if report.lost > 0 {
                    logger.error("\(report.lost) blobs are on no server")
                }
                for failure in report.failed {
                    logger.error("Copying a blob failed: \(failure, privacy: .public)")
                }
            } catch CoreError.Cancelled {
            } catch {
                logger.error("Copying blobs failed: \(error.localizedDescription, privacy: .public)")
            }
            fill = nil
            filling = nil
            fillStop = nil
            info = try? self.local?.info()
        }
    }

    /// Deletes the blobs no version referenced for 30 days, once a day, once
    /// the session synced every relay. Never while a fill copies blobs.
    fileprivate func startCollection() {
        guard collecting == nil, filling == nil, let local,
              lastCollection.map({ $0.timeIntervalSinceNow < -86400 }) ?? true,
              let id = info?.id, let secret = try? DriveKeychain.secret(drive: id)
        else { return }
        let stop = Cancellation()
        collectionStop = stop
        lastCollection = Date()
        collecting = Task {
            do {
                let report = try await LupineCore.collectBlobs(drive: local, secret: secret, cancellation: stop)
                if report.deleted > 0 {
                    logger.info("Collection deleted \(report.deleted) blobs")
                }
                if report.heldBack {
                    logger.info("Collection held back: the drive holds a version this app cannot read")
                }
                for failure in report.failed {
                    logger.error("Deleting a blob failed: \(failure, privacy: .public)")
                }
            } catch CoreError.Cancelled {
            } catch {
                logger.error("Collecting blobs failed: \(error.localizedDescription, privacy: .public)")
            }
            collecting = nil
            collectionStop = nil
        }
    }

    fileprivate func fillProgressed(_ progress: Fill) {
        if filling != nil {
            fill = progress
        }
    }

    fileprivate func compacted(_ wraps: UInt64) {
        if wraps > 0 {
            logger.info("Compaction deleted \(wraps) wraps")
        }
    }

    /// The drive changed: Finder and the synced folders should look.
    fileprivate func driveChanged() {
        signalFinder()
        folders?.changed()
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

    private func refreshLoginItem() {
        let status = SMAppService.mainApp.status
        opensAtLogin = status == .enabled
        loginItemNeedsApproval = status == .requiresApproval
    }

    private func registerDomain() async {
        domainState = .registering
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
        onMain {
            $0.report(.synced(failedRelays: failed.count))
            // Folder sync waits for the first full sync, which may change no item.
            $0.folders?.changed()
            $0.keepDescriptor()
            if failed.isEmpty {
                $0.startFill()
                $0.startCollection()
            }
        }
    }

    func listsChanged() {
        onMain { model in
            Task { await model.reopenDrive() }
        }
    }

    func compacted(wraps: UInt64) {
        onMain { $0.compacted(wraps) }
    }

    func itemsChanged() {
        onMain { $0.driveChanged() }
    }

    private func onMain(_ update: @escaping @MainActor @Sendable (AppModel) -> Void) {
        let model = model
        DispatchQueue.main.async {
            MainActor.assumeIsolated { update(model) }
        }
    }
}

private final class FillReporter: FillObserver {
    private let model: AppModel

    init(model: AppModel) {
        self.model = model
    }

    func progressed(done: UInt64, total: UInt64) {
        let model = model
        DispatchQueue.main.async {
            MainActor.assumeIsolated { model.fillProgressed(AppModel.Fill(done: done, total: total)) }
        }
    }
}
