import SwiftUI

@main
struct LupineApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        MenuBarExtra("Lupine", systemImage: "externaldrive.badge.icloud") {
            StatusView(model: model)
        }
        .menuBarExtraStyle(.window)
    }
}
