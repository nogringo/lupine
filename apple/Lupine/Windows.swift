import AppKit
import SwiftUI

/// The app's windows. A menu bar agent has no main window, and SwiftUI
/// scenes neither bring it forward nor open at launch reliably.
@MainActor
final class Windows: NSObject, NSWindowDelegate {
    enum ID {
        case onboarding
        case settings
    }

    private var open: [ID: NSWindow] = [:]
    private var closed: [ID: () -> Void] = [:]

    func show(_ id: ID, title: String, onClose: (() -> Void)? = nil, content: () -> some View) {
        NSApp.activate()
        if let window = open[id] {
            window.makeKeyAndOrderFront(nil)
            return
        }
        let controller = NSHostingController(rootView: content())
        let window = NSWindow(contentViewController: controller)
        window.title = title
        window.styleMask = [.titled, .closable]
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.center()
        open[id] = window
        closed[id] = onClose
        window.makeKeyAndOrderFront(nil)
    }

    func close(_ id: ID) {
        open[id]?.close()
    }

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              let id = open.first(where: { $0.value === window })?.key
        else { return }
        open[id] = nil
        closed.removeValue(forKey: id)?()
    }
}
