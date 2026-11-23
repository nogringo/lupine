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
