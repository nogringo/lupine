import AppKit
import LupineCore
import SwiftUI

struct SettingsView: View {
    let model: AppModel
    @State private var recoveryKey: String?
    @State private var confirmsSignOut = false
    @State private var unsent: UInt64 = 0
    @State private var editsLists = false

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
                    if !info.leaving.isEmpty {
                        LabeledContent("Leaving") { AddressList(addresses: info.leaving) }
                    }
                    if let fill = model.fill {
                        LabeledContent("Copying files to the servers") {
                            Text(fill.total == 0 ? "Listing" : "\(fill.done) of \(fill.total)")
                        }
                        if fill.total > 0 {
                            ProgressView(value: Double(fill.done), total: Double(fill.total))
                        }
                    } else if info.filling {
                        Text("Files will be copied to the servers after the next sync with every relay.")
                            .foregroundStyle(.secondary)
                    }
                    HStack {
                        Spacer()
                        Button("Edit Relays and Servers...") { editsLists = true }
                    }
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
        .sheet(isPresented: $editsLists) {
            if let info = model.info {
                ListsEditor(model: model, info: info)
            }
        }
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

private struct ListsEditor: View {
    let model: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var relays: String
    @State private var servers: String
    @State private var saving = false
    @State private var error: String?

    init(model: AppModel, info: DriveInfo) {
        self.model = model
        _relays = State(initialValue: info.relays.joined(separator: "\n"))
        _servers = State(initialValue: info.servers.joined(separator: "\n"))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Relays and Servers").font(.title3.bold())

            VStack(alignment: .leading, spacing: 4) {
                Text("Relays, one per line").font(.headline)
                ListEditor(text: $relays)
                Text("A new relay gets the whole drive at the next sync.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 4) {
                Text("Blossom servers, one per line, most trusted first").font(.headline)
                ListEditor(text: $servers)
                Text("A new server gets a copy of every file. Lupine reads from the servers you remove until the copy is done, then leaves their files in place.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if let error {
                Text(error)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack {
                if saving {
                    ProgressView().controlSize(.small)
                }
                Spacer()
                Button("Cancel") { dismiss() }
                    .disabled(saving)
                Button("Save", action: save)
                    .keyboardShortcut(.defaultAction)
                    .disabled(saving || ListEditor.entries(relays).isEmpty || ListEditor.entries(servers).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 440)
    }

    private func save() {
        saving = true
        error = nil
        Task {
            do {
                try await model.changeLists(
                    relays: ListEditor.entries(relays),
                    servers: ListEditor.entries(servers)
                )
                dismiss()
            } catch {
                self.error = error.localizedDescription
                saving = false
            }
        }
    }
}
