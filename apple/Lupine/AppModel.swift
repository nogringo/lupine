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
        case rotating
        case failed(String)
    }

    struct RotationProgress: Equatable, Sendable {
        var label: String
        var done: UInt64 = 0
        var total: UInt64 = 0
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
    /// Where the key rotation under way stands.
    private(set) var rotation: RotationProgress?
    /// Another device rotated the drive key: the user must sign in again.
    private(set) var retired = false
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
    private var retiring: Task<Void, Never>?
    private var lastRetirement: Date?

    var hasDrive: Bool {
        local != nil || driveState == .rotating
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
        let current = try openLocalDrive(path: path).flatMap { try DriveKeychain.secret(drive: $0.info().id) }
        try await setUp(try await LupineCore.importRecoveryKey(path: path, recoveryKey: recoveryKey, current: current))
    }

    /// The database is set up for `drive`: keep its secret and open it. A
    /// secret it replaces waits to retire its key.
    func setUp(_ drive: ImportedDrive) async throws {
        if let old = try DriveKeychain.secret(drive: drive.id), old != drive.secret {
            try DriveKeychain.store(secret: old, drive: drive.id, role: .previous)
        }
        try DriveKeychain.store(secret: drive.secret, drive: drive.id)
        retired = false
        await openDrive()
    }

    /// Moves the drive to a new key, which the user signed in as `signIn`
    /// signs. Returns the new recovery key.
    func rotateKey(signIn: SignIn) async throws -> String {
        guard let local, let id = info?.id, let secret = try DriveKeychain.secret(drive: id) else {
            throw CoreError.Check(message: "No drive is open.")
        }
        let next = try await signIn.prepareRotation(drive: local, secret: secret)
        try DriveKeychain.store(secret: next, drive: id, role: .next)
        info = try local.info()
        return try await finishRotation(signIn: signIn)
    }

    /// Finishes the rotation this Mac started and did not finish.
    func continueRotation() async throws -> String {
        try await finishRotation(signIn: nil)
    }

    /// Gives up the rotation this Mac started, until it announced the new key.
    func cancelRotation() async {
        guard let local, let id = info?.id else { return }
        do {
            let next = try DriveKeychain.secret(drive: id, role: .next)
            try await abandonRotation(drive: local, nextSecret: next)
            try DriveKeychain.delete(drive: id, role: .next)
        } catch {
            logger.error("Cancelling the key rotation failed: \(error.localizedDescription, privacy: .public)")
        }
        info = try? local.info()
    }

    /// Syncing stops while the drive moves to the new key, then the drive
    /// opens again, on the new key once the rotation committed.
    private func finishRotation(signIn: SignIn?) async throws -> String {
        guard let local, let id = info?.id, let secret = try DriveKeychain.secret(drive: id),
              let next = try DriveKeychain.secret(drive: id, role: .next)
        else {
            throw CoreError.Check(message: "No key rotation is under way.")
        }
        await closeDrive()
        driveState = .rotating
        rotation = RotationProgress(label: "Preparing")
        defer { rotation = nil }
        do {
            let report = try await rotateDriveKey(
                drive: local, secret: secret, nextSecret: next, signIn: signIn, observer: RotationReporter(model: self)
            )
            if report.unclaimed > 0 {
                logger.info("\(report.unclaimed) blob copies stay under the previous drive key")
            }
            await openDrive()
            return report.recoveryKey
        } catch {
            await openDrive()
            throw error
        }
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
                for role in [DriveKeychain.Role.current, .next, .previous] {
                    try DriveKeychain.delete(drive: id, role: role)
                }
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
        lastRetirement = nil
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
            guard let local = try openLocalDrive(path: path) else {
                driveState = .none
                return
            }
            var info = try local.info()
            self.info = info
            if info.nextDriveKey != nil {
                do {
                    info = try await pendingRotation(local, info)
                } catch {
                    // Settings can continue it.
                    self.local = local
                    throw error
                }
                self.info = info
            }
            guard let secret = try driveSecret(info) else {
                driveState = .none
                return
            }
            if info.retiringDriveKey == nil {
                try DriveKeychain.delete(drive: info.id, role: .previous)
            }
            self.local = local
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

    /// The secret that opens the drive. Once a rotation moved the database
    /// to the new key, the new secret becomes current, and the old one
    /// waits to retire its key.
    private func driveSecret(_ info: DriveInfo) throws -> String? {
        let opens = { (secret: String) in (try? LupineCore.driveKey(secret: secret)) == info.driveKey }
        let current = try DriveKeychain.secret(drive: info.id)
        if let current, opens(current) {
            return current
        }
        guard let next = try DriveKeychain.secret(drive: info.id, role: .next), opens(next) else {
            return nil
        }
        if let current {
            try DriveKeychain.store(secret: current, drive: info.id, role: .previous)
        }
        try DriveKeychain.store(secret: next, drive: info.id)
        try DriveKeychain.delete(drive: info.id, role: .next)
        return next
    }

    /// A rotation this Mac started: one that announced the new key finishes
    /// before the drive opens, one whose new secret is lost is given up, and
    /// any other waits for Settings.
    private func pendingRotation(_ local: LocalDrive, _ info: DriveInfo) async throws -> DriveInfo {
        let next = try DriveKeychain.secret(drive: info.id, role: .next)
        guard info.rotationCommitted else {
            if next == nil {
                try await abandonRotation(drive: local, nextSecret: nil)
            }
            return try local.info()
        }
        guard let next, let current = try DriveKeychain.secret(drive: info.id) else {
            return info
        }
        driveState = .rotating
        rotation = RotationProgress(label: "Finishing the key change")
        defer { rotation = nil }
        _ = try await rotateDriveKey(
            drive: local, secret: current, nextSecret: next, signIn: nil, observer: RotationReporter(model: self)
        )
        return try local.info()
    }

    fileprivate func rotationProgressed(_ progress: RotationProgress) {
        if rotation != nil {
            rotation = progress
        }
    }

    /// Another device rotated the drive key: signing in again moves this
    /// Mac to the new one.
    fileprivate func driveRetired() {
        Task {
            await closeDrive()
            retired = true
            showOnboarding()
        }
    }

    /// Deletes the wraps of the key a rotation retired, once the session
    /// synced every relay. Tries again after an hour.
    fileprivate func startRetirement() {
        guard retiring == nil, let local, let id = info?.id, info?.retiringDriveKey != nil,
              lastRetirement.map({ $0.timeIntervalSinceNow < -3600 }) ?? true
        else { return }
        let previous = try? DriveKeychain.secret(drive: id, role: .previous)
        lastRetirement = Date()
        retiring = Task {
            do {
                if try await retireDriveKey(drive: local, previousSecret: previous) {
                    try DriveKeychain.delete(drive: id, role: .previous)
                }
            } catch {
                logger.error("Retiring the previous drive key failed: \(error.localizedDescription, privacy: .public)")
            }
            retiring = nil
            info = try? self.local?.info()
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
                $0.startRetirement()
            }
        }
    }

    func retired() {
        onMain { $0.driveRetired() }
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

private final class RotationReporter: RotationObserver {
    private let model: AppModel

    init(model: AppModel) {
        self.model = model
    }

    func progressed(stage: RotationStage) {
        let progress = switch stage {
        case .lists: AppModel.RotationProgress(label: "Announcing the new key")
        case .claiming(let done, let total):
            AppModel.RotationProgress(label: "Asking the servers to keep the files", done: done, total: total)
        case .wrapping(let done, let total):
            AppModel.RotationProgress(label: "Encrypting the drive for the new key", done: done, total: total)
        case .sending: AppModel.RotationProgress(label: "Sending it to the relays")
        case .publishing: AppModel.RotationProgress(label: "Publishing the new key")
        case .moving: AppModel.RotationProgress(label: "Updating this Mac")
        }
        let model = model
        DispatchQueue.main.async {
            MainActor.assumeIsolated { model.rotationProgressed(progress) }
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
