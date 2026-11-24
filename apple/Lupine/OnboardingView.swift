import AppKit
import CoreImage.CIFilterBuiltins
import LupineCore
import SwiftUI

struct OnboardingView: View {
    @Bindable var onboarding: Onboarding

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            content
            if let error = onboarding.error {
                Text(error)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    @ViewBuilder
    private var content: some View {
        switch onboarding.step {
        case .welcome:
            WelcomeStep(onboarding: onboarding)
        case .pairing(let link):
            PairingStep(link: link) { onboarding.back() }
        case .working(let label):
            HStack(spacing: 12) {
                ProgressView().controlSize(.small)
                Text("\(label)...")
            }
            Text("A remote signer may ask you to approve Lupine.")
                .font(.callout)
                .foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Back") { onboarding.back() }
            }
        case .chooseDrive:
            ChooseDriveStep(onboarding: onboarding)
        case .createDrive:
            CreateDriveStep(onboarding: onboarding)
        case .recoveryKey(let key):
            RecoveryKeyStep(key: key) { onboarding.savedRecoveryKey() }
        case .done:
            DoneStep(onboarding: onboarding)
        }
    }
}

private struct WelcomeStep: View {
    let onboarding: Onboarding
    @State private var key = ""
    @State private var recoveryKey = ""
    @State private var showsRecovery = false

    var body: some View {
        Text("Welcome to Lupine").font(.title2.bold())
        Text("An end-to-end encrypted drive on Nostr. Sign in with your Nostr account to open your drive, or to create one.")
            .fixedSize(horizontal: false, vertical: true)

        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                Text("Use a signer app on your phone or computer (NIP-46).")
                Button("Connect a Signer...") { onboarding.pairSigner() }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(4)
        }

        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                Text("Or paste a bunker:// link, or your private key (nsec).")
                HStack {
                    SecureField("bunker://... or nsec1...", text: $key)
                        .onSubmit(signIn)
                    Button("Sign In", action: signIn)
                        .disabled(key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                Text("Your key only opens the drive: Lupine then keeps the drive's own key, never yours.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(4)
        }

        DisclosureGroup("Open with a recovery key", isExpanded: $showsRecovery) {
            HStack {
                SecureField("lupine1...", text: $recoveryKey)
                    .onSubmit(openRecoveryKey)
                Button("Open", action: openRecoveryKey)
                    .disabled(recoveryKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(.top, 4)
        }
    }

    private func signIn() {
        let key = key
        self.key = ""
        onboarding.signIn(key: key)
    }

    private func openRecoveryKey() {
        let key = recoveryKey
        recoveryKey = ""
        onboarding.useRecoveryKey(key)
    }
}

private struct PairingStep: View {
    let link: String
    let back: () -> Void

    var body: some View {
        Text("Connect a Signer").font(.title2.bold())
        Text("Scan this code with your signer app, or copy the link into it, then approve Lupine.")
            .fixedSize(horizontal: false, vertical: true)
        HStack {
            Spacer()
            if let code = QRCode.image(link) {
                Image(nsImage: code)
                    .interpolation(.none)
                    .resizable()
                    .frame(width: 220, height: 220)
            }
            Spacer()
        }
        HStack(spacing: 12) {
            ProgressView().controlSize(.small)
            Text("Waiting for your signer...")
            Spacer()
            Button("Copy Link") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(link, forType: .string)
            }
            Button("Back", action: back)
        }
    }
}

private struct ChooseDriveStep: View {
    let onboarding: Onboarding

    var body: some View {
        Text("Open Your Drive").font(.title2.bold())
        if let user = onboarding.user {
            Text("Signed in as \(Abbreviated.key(user)).")
                .foregroundStyle(.secondary)
                .help(user)
        }
        VStack(spacing: 8) {
            ForEach(onboarding.drives, id: \.id) { drive in
                HStack {
                    Image(systemName: "externaldrive")
                    Text(drive.name)
                    Spacer()
                    Button("Open") { onboarding.open(drive) }
                }
            }
        }
        HStack {
            Button("Back") { onboarding.back() }
            Spacer()
            Button("Create a New Drive...") { onboarding.showCreation() }
        }
    }
}

private struct CreateDriveStep: View {
    @Bindable var onboarding: Onboarding

    var body: some View {
        Text("Create a Drive").font(.title2.bold())
        if let user = onboarding.user {
            Text("You have no drive yet as \(Abbreviated.key(user)).")
                .foregroundStyle(.secondary)
                .help(user)
        }
        TextField("Name", text: $onboarding.name)

        VStack(alignment: .leading, spacing: 4) {
            Text("Relays, one per line").font(.headline)
            ListEditor(text: $onboarding.relays)
            Text("They store the drive's encrypted metadata. Prefer relays you run or pay for, that serve gift wraps only to their recipient (NIP-17).")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }

        VStack(alignment: .leading, spacing: 4) {
            Text("Blossom servers, one per line").font(.headline)
            ListEditor(text: $onboarding.servers)
            Text("They store the encrypted files. Media hosts refuse them: Lupine checks each server first.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }

        HStack {
            Button("Back") { onboarding.back() }
            Spacer()
            Button("Create Drive") { onboarding.create() }
                .keyboardShortcut(.defaultAction)
                .disabled(!onboarding.canCreate)
        }
    }
}

struct ListEditor: View {
    @Binding var text: String

    /// The addresses typed, one per line.
    static func entries(_ text: String) -> [String] {
        text.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
    }

    var body: some View {
        TextEditor(text: $text)
            .font(.body.monospaced())
            .autocorrectionDisabled()
            .frame(height: 64)
            .scrollContentBackground(.hidden)
            .padding(4)
            .background(.background, in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
    }
}

struct RecoveryKeyStep: View {
    let key: String
    let saved: () -> Void

    var body: some View {
        Text("Save Your Recovery Key").font(.title2.bold())
        Text("It opens the drive without your Nostr key, if you lose access to it. Anyone who has it can read and change the whole drive: keep it somewhere safe, such as a password manager.")
            .fixedSize(horizontal: false, vertical: true)
        RecoveryKeyText(key: key)
        HStack {
            Spacer()
            Button("I Saved It", action: saved)
                .keyboardShortcut(.defaultAction)
        }
    }
}

struct RecoveryKeyText: View {
    let key: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(key)
                .font(.callout.monospaced())
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
            Button("Copy") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(key, forType: .string)
            }
        }
    }
}

private struct DoneStep: View {
    @Bindable var onboarding: Onboarding

    var body: some View {
        Text("Lupine Is Ready").font(.title2.bold())
        Text("Your drive is in the Finder sidebar. If it does not show up, turn on the Lupine extension in System Settings, General, Login Items & Extensions.")
            .fixedSize(horizontal: false, vertical: true)
        Text("To sync a folder of this Mac with the drive, choose Add Folder... in the Lupine menu.")
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        Toggle("Open Lupine at login", isOn: $onboarding.openAtLogin)
        HStack {
            Spacer()
            Button("Done") { onboarding.finish() }
                .keyboardShortcut(.defaultAction)
        }
    }
}

enum Abbreviated {
    /// `npub1abcdef...uvwxyz`
    static func key(_ key: String) -> String {
        guard key.count > 20 else { return key }
        return "\(key.prefix(11))...\(key.suffix(6))"
    }
}

enum QRCode {
    static func image(_ text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let code = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else {
            return nil
        }
        let representation = NSCIImageRep(ciImage: code)
        let image = NSImage(size: representation.size)
        image.addRepresentation(representation)
        return image
    }
}
