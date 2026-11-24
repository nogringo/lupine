import AppKit
import LupineCore
import SwiftUI

struct SettingsView: View {
    let model: AppModel
    @State private var recoveryKey: String?
    @State private var confirmsSignOut = false
    @State private var unsent: UInt64 = 0

    var body: some View {
        Form {
            if let info = model.info {
                Section("Account") {
                    if let user = info.user {
                        KeyRow(label: "Signed in as", key: user)
                    } else {
                        Text("Opened with a recovery key")
                    }
                }
                Section {
                    LabeledContent("Name", value: info.name ?? "Lupine")
                    KeyRow(label: "Drive key", key: info.driveKey)
                    LabeledContent("Relays") { AddressList(addresses: info.relays) }
                    LabeledContent("Blossom servers") { AddressList(addresses: info.servers) }
                } header: {
                    Text("Drive")
                } footer: {
                    Text("Relays and servers see the drive key, not yours. Paid ones must allow it.")
                        .foregroundStyle(.secondary)
                }
                Section {
                    Picker("Keep replaced versions for", selection: Binding(
                        get: { model.historyDays },
                        set: { model.setHistoryDays($0) }
                    )) {
                        Text("1 week").tag(UInt32(7))
                        Text("1 month").tag(UInt32(30))
                        Text("3 months").tag(UInt32(90))
                        Text("1 year").tag(UInt32(365))
                    }
                } header: {
                    Text("History")
                } footer: {
                    Text("Older ones are deleted from the relays within a day. Each device of the drive applies its own setting, so the shortest one wins.")
                        .foregroundStyle(.secondary)
                }
                Section {
                    if let recoveryKey {
                        RecoveryKeyText(key: recoveryKey)
                        Button("Hide") { self.recoveryKey = nil }
                    } else {
                        Button("Show Recovery Key") { recoveryKey = model.recoveryKey() }
                    }
                } header: {
                    Text("Recovery Key")
                } footer: {
                    Text("Opens the drive without your Nostr key. Anyone who has it can read and change everything.")
                        .foregroundStyle(.secondary)
                }
            }
            Section("General") {
                Toggle("Open Lupine at login", isOn: Binding(
                    get: { model.opensAtLogin },
                    set: { model.setOpenAtLogin($0) }
                ))
                if model.loginItemNeedsApproval {
                    Text("Allow Lupine in System Settings, General, Login Items & Extensions.")
                        .foregroundStyle(.secondary)
                }
            }
            Section("About") {
                LabeledContent("Core", value: model.coreVersion)
                LabeledContent("Self-test", value: model.selfTestResult ?? "Running")
            }
            if model.info != nil {
                Section {
                    HStack {
                        Spacer()
                        Button("Sign Out...", role: .destructive) {
                            unsent = model.unsentChanges()
                            confirmsSignOut = true
                        }
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 480, height: 620)
        .alert("Sign out of Lupine on this Mac?", isPresented: $confirmsSignOut) {
            Button("Sign Out", role: .destructive) {
                Task { await model.signOut() }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(signOutMessage)
        }
    }

    private var signOutMessage: String {
        var message = "The drive leaves Finder and synced folders stop syncing. Their files stay on this Mac, and the drive stays on its relays: sign in again, or use the recovery key, to open it."
        if unsent == 1 {
            message += "\n\n1 change has not reached the relays yet and will be lost."
        } else if unsent > 1 {
            message += "\n\n\(unsent) changes have not reached the relays yet and will be lost."
        }
        return message
    }
}

private struct KeyRow: View {
    let label: String
    let key: String

    var body: some View {
        LabeledContent(label) {
            HStack(spacing: 6) {
                Text(Abbreviated.key(key))
                    .font(.body.monospaced())
                    .help(key)
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(key, forType: .string)
                } label: {
                    Image(systemName: "doc.on.doc")
                }
                .buttonStyle(.borderless)
                .help("Copy")
            }
        }
    }
}

private struct AddressList: View {
    let addresses: [String]

    var body: some View {
        VStack(alignment: .trailing, spacing: 2) {
            ForEach(addresses, id: \.self) { address in
                Text(address).textSelection(.enabled)
            }
        }
    }
}
