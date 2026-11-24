import LupineCore
import Observation
import SwiftUI

/// Changing the drive key: the user signs in once to sign the new key, then
/// the drive moves to it.
@MainActor
@Observable
final class KeyRotation {
    enum Step: Equatable {
        case intro
        case signIn
        case pairing(link: String)
        case working
        case done(recoveryKey: String)
        case failed(String)
    }

    private(set) var step: Step
    private(set) var error: String?
    let app: AppModel
    private let resuming: Bool
    private var started = false
    private var pairing: SignerPairing?
    private var work: Task<Void, Never>?

    /// `resuming` finishes the rotation this Mac started, without signing in.
    init(app: AppModel, resuming: Bool) {
        self.app = app
        self.resuming = resuming
        step = resuming ? .working : .intro
    }

    var isWorking: Bool {
        step == .working
    }

    func start() {
        guard resuming, !started else { return }
        started = true
        work = Task {
            do {
                step = .done(recoveryKey: try await app.continueRotation())
            } catch {
                step = .failed(error.localizedDescription)
            }
        }
    }

    func showSignIn() {
        error = nil
        step = .signIn
    }

    /// An nsec or a `bunker://` link.
    func signIn(key: String) {
        error = nil
        work = Task {
            do {
                await rotate(with: try await LupineCore.signIn(key: key, approval: SignerPage()))
            } catch where !Task.isCancelled {
                self.error = error.localizedDescription
            } catch {}
        }
    }

    func pairSigner() {
        error = nil
        let pairing: SignerPairing
        do {
            pairing = try LupineCore.pairSigner(approval: SignerPage())
        } catch {
            self.error = error.localizedDescription
            return
        }
        self.pairing = pairing
        step = .pairing(link: pairing.link())
        work = Task {
            do {
                await rotate(with: try await pairing.accepted())
            } catch where !Task.isCancelled {
                self.error = error.localizedDescription
                step = .signIn
            } catch {}
        }
    }

    func back() {
        stopSignIn()
        error = nil
        step = .signIn
    }

    /// The sheet closed. A rotation under way goes on.
    func cancel() {
        if !isWorking {
            stopSignIn()
        }
    }

    private func stopSignIn() {
        work?.cancel()
        work = nil
        pairing?.cancel()
        pairing = nil
    }

    private func rotate(with session: SignIn) async {
        pairing = nil
        step = .working
        do {
            step = .done(recoveryKey: try await app.rotateKey(signIn: session))
        } catch {
            step = .failed(error.localizedDescription)
        }
        await session.close()
    }
}

struct KeyRotationView: View {
    @State private var rotation: KeyRotation
    @Environment(\.dismiss) private var dismiss
    @State private var key = ""

    init(app: AppModel, resuming: Bool) {
        _rotation = State(initialValue: KeyRotation(app: app, resuming: resuming))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            content
            if let error = rotation.error {
                Text(error)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(20)
        .frame(width: 440)
        .interactiveDismissDisabled(rotation.isWorking)
        .task { rotation.start() }
        .onDisappear { rotation.cancel() }
    }

    @ViewBuilder
    private var content: some View {
        switch rotation.step {
        case .intro:
            Text("Change the Drive Key").font(.title3.bold())
            Text("A new key locks out a lost device and the current recovery key: neither can read what changes from then on. Your other devices will ask you to sign in again, and you get a new recovery key.")
                .fixedSize(horizontal: false, vertical: true)
            Text("Lupine needs your Nostr key once, to sign the new key. Syncing pauses until the change is done.")
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Continue") { rotation.showSignIn() }
                    .keyboardShortcut(.defaultAction)
            }
        case .signIn:
            Text("Sign In to Sign the New Key").font(.title3.bold())
            if let user = rotation.app.info?.user {
                Text("Sign in as \(Abbreviated.key(user)).")
                    .foregroundStyle(.secondary)
                    .help(user)
            }
            Button("Connect a Signer...") { rotation.pairSigner() }
            HStack {
                SecureField("bunker://... or nsec1...", text: $key)
                    .onSubmit(signIn)
                Button("Sign In", action: signIn)
                    .disabled(key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
            }
        case .pairing(let link):
            PairingStep(link: link) { rotation.back() }
        case .working:
            Text("Changing the Drive Key").font(.title3.bold())
            if let progress = rotation.app.rotation {
                if progress.total > 0 {
                    ProgressView(value: Double(progress.done), total: Double(progress.total)) {
                        Text(progress.label)
                    }
                } else {
                    HStack(spacing: 12) {
                        ProgressView().controlSize(.small)
                        Text("\(progress.label)...")
                    }
                }
            } else {
                ProgressView().controlSize(.small)
            }
            Text("Keep Lupine open until the change is done. If it quits, it finishes the change when it opens again.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        case .done(let recoveryKey):
            Text("The drive has a new key. Your other devices will ask you to sign in again, and the previous recovery key no longer opens the drive.")
                .fixedSize(horizontal: false, vertical: true)
            RecoveryKeyStep(key: recoveryKey) { dismiss() }
        case .failed(let message):
            Text("The Key Change Did Not Finish").font(.title3.bold())
            Text(message)
                .foregroundStyle(.red)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Close") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    private func signIn() {
        let key = key
        self.key = ""
        rotation.signIn(key: key)
    }
}
