import AppKit
import LupineCore
import Observation

/// Signing in and setting up the drive of this Mac: the user's key opens a
/// drive or creates one, then only the drive's key stays on the Mac.
@MainActor
@Observable
final class Onboarding {
    enum Step: Equatable {
        case welcome
        case pairing(link: String)
        case working(String)
        case chooseDrive
        case createDrive
        case recoveryKey(String)
        case done
    }

    private(set) var step = Step.welcome
    private(set) var error: String?
    private(set) var user: String?
    private(set) var drives: [RemoteDrive] = []
    var name = "Lupine"
    var relays = ""
    var servers = ""
    var openAtLogin = true

    private let app: AppModel
    private var session: SignIn?
    private var pairing: SignerPairing?
    private var work: Task<Void, Never>?

    init(app: AppModel) {
        self.app = app
    }

    var canCreate: Bool {
        !Self.lines(relays).isEmpty && !Self.lines(servers).isEmpty
    }

    /// An nsec or a `bunker://` link.
    func signIn(key: String) {
        perform("Signing in", failing: .welcome) {
            try await self.signedIn(try await LupineCore.signIn(key: key, approval: SignerPage()))
        }
    }

    func pairSigner() {
        let pairing: SignerPairing
        do {
            pairing = try LupineCore.pairSigner(approval: SignerPage())
        } catch {
            fail(error, back: .welcome)
            return
        }
        self.pairing = pairing
        perform(.pairing(link: pairing.link()), failing: .welcome) {
            try await self.signedIn(try await pairing.accepted())
        }
    }

    func useRecoveryKey(_ key: String) {
        perform("Opening the drive", failing: .welcome) {
            try await self.app.importRecoveryKey(key)
            self.step = .done
        }
    }

    func open(_ drive: RemoteDrive) {
        guard let session else { return }
        perform("Opening \(drive.name)", failing: .chooseDrive) {
            let path = try SharedContainer.databaseURL().path(percentEncoded: false)
            let opened = try await session.openDrive(path: path, id: drive.id)
            try Task.checkCancellation()
            try await self.app.setUp(opened)
            await self.closeSession()
            self.step = .done
        }
    }

    func showCreation() {
        guard let session else { return }
        perform("Looking for your relays", failing: .welcome) {
            let suggested = await session.suggestedLists()
            try Task.checkCancellation()
            if self.relays.isEmpty {
                self.relays = suggested.relays.joined(separator: "\n")
            }
            if self.servers.isEmpty {
                self.servers = suggested.servers.joined(separator: "\n")
            }
            self.step = .createDrive
        }
    }

    func create() {
        guard let session else { return }
        let (name, relays, servers) = (name, Self.lines(relays), Self.lines(servers))
        perform("Creating the drive", failing: .createDrive) {
            let path = try SharedContainer.databaseURL().path(percentEncoded: false)
            let created = try await session.createDrive(path: path, name: name, relays: relays, servers: servers)
            try Task.checkCancellation()
            try await self.app.setUp(created)
            await self.closeSession()
            self.step = .recoveryKey(created.recoveryKey)
        }
    }

    func savedRecoveryKey() {
        step = .done
    }

    func back() {
        stop()
        error = nil
        drives = []
        user = nil
        step = .welcome
    }

    func finish() {
        if openAtLogin {
            app.setOpenAtLogin(true)
        }
        app.windows.close(.onboarding)
    }

    /// The window closed.
    func cancel() {
        stop()
    }

    private func stop() {
        work?.cancel()
        work = nil
        pairing?.cancel()
        pairing = nil
        Task { await closeSession() }
    }

    private func perform(_ label: String, failing back: Step, _ body: @escaping @MainActor () async throws -> Void) {
        perform(.working(label), failing: back, body)
    }

    /// Runs `body` until it ends or the user goes back, which cancels it.
    private func perform(_ waiting: Step, failing back: Step, _ body: @escaping @MainActor () async throws -> Void) {
        work?.cancel()
        error = nil
        step = waiting
        work = Task {
            do {
                try await body()
            } catch let error where !Task.isCancelled {
                fail(error, back: back)
            } catch {}
        }
    }

    private func signedIn(_ session: SignIn) async throws {
        pairing = nil
        guard !Task.isCancelled else {
            await session.close()
            throw CancellationError()
        }
        self.session = session
        user = session.user()
        step = .working("Looking for your drive")
        let drives = try await session.drives()
        try Task.checkCancellation()
        self.drives = drives
        if drives.isEmpty {
            showCreation()
        } else {
            step = .chooseDrive
        }
    }

    private func closeSession() async {
        let session = session
        self.session = nil
        await session?.close()
    }

    private func fail(_ error: Error, back: Step) {
        self.error = error.localizedDescription
        step = back
        if back == .welcome {
            Task { await closeSession() }
        }
    }

    private static func lines(_ text: String) -> [String] {
        text.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
    }
}

/// Opens the page where a remote signer asks the user to approve Lupine.
private final class SignerPage: SignerApproval, Sendable {
    func openUrl(url: String) {
        guard let url = URL(string: url) else { return }
        DispatchQueue.main.async {
            NSWorkspace.shared.open(url)
        }
    }
}
