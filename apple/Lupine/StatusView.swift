import SwiftUI

struct StatusView: View {
    let model: AppModel
    @State private var recoveryKey = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Lupine").font(.headline)
            LabeledContent("Core", value: model.coreVersion)
            LabeledContent("Finder", value: domainLabel)
            LabeledContent("Drive", value: driveLabel)
            if model.canOpenDrive {
                SecureField("Recovery key", text: $recoveryKey)
                Button("Open Drive") {
                    let key = recoveryKey
                    recoveryKey = ""
                    Task { await model.importRecoveryKey(key) }
                }
                .disabled(recoveryKey.isEmpty)
            }
            if let folders = model.folders {
                Divider()
                FoldersSection(folders: folders)
            }
            Divider()
            LabeledContent("Self-test", value: model.selfTestResult ?? "Running")
            Divider()
            Button("Quit Lupine") { NSApplication.shared.terminate(nil) }
        }
        .padding()
        .frame(width: 300)
    }

    private var domainLabel: String {
        switch model.domainState {
        case .registering: "Registering"
        case .registered: "Ready"
        case .failed(let message): message
        }
    }

    private var driveLabel: String {
        switch model.driveState {
        case .none: "None"
        case .opening: "Opening"
        case .syncing: "Syncing"
        case .synced(0): "Up to date"
        case .synced(let failed): "\(failed) relays unreachable"
        case .failed(let message): message
        }
    }
}

private struct FoldersSection: View {
    let folders: SyncedFolders

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Synced Folders").font(.subheadline)
            ForEach(folders.rows) { row in
                HStack {
                    VStack(alignment: .leading) {
                        Text(row.name)
                        Text(label(row.state))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .help(row.path)
                    Spacer()
                    Button {
                        folders.remove(row.id)
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.borderless)
                    .help("Stop syncing this folder. It stays on this Mac and in the drive.")
                }
            }
            Button("Add Folder...") { folders.add() }
            if let error = folders.error {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }

    private func label(_ state: SyncedFolders.State) -> String {
        switch state {
        case .waiting: "Waiting for the drive"
        case .syncing: "Syncing"
        case .synced(0): "Up to date"
        case .synced(1): "1 item not synced yet"
        case .synced(let failed): "\(failed) items not synced yet"
        case .failed(let message): message
        }
    }
}
