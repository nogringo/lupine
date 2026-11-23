import SwiftUI

struct StatusView: View {
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Lupine").font(.headline)
            LabeledContent("Core", value: model.coreVersion)
            LabeledContent("Finder", value: domainLabel)
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
}
