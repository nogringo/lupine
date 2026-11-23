import Foundation

/// The File Provider extension writes Finder's changes to the local
/// database, and posts this for the app to send them to the relays.
enum OutboxSignal {
    private static let name = CFNotificationName("\(SharedContainer.appGroup).outbox" as CFString)

    static func post() {
        CFNotificationCenterPostNotification(CFNotificationCenterGetDarwinNotifyCenter(), name, nil, nil, true)
    }

    /// Calls `action` on each post until the listener is released.
    final class Listener: Sendable {
        private let action: @Sendable () -> Void

        init(_ action: @escaping @Sendable () -> Void) {
            self.action = action
            CFNotificationCenterAddObserver(
                CFNotificationCenterGetDarwinNotifyCenter(),
                Unmanaged.passUnretained(self).toOpaque(),
                { _, observer, _, _, _ in
                    guard let observer else { return }
                    Unmanaged<Listener>.fromOpaque(observer).takeUnretainedValue().action()
                },
                OutboxSignal.name.rawValue,
                nil,
                .deliverImmediately
            )
        }

        deinit {
            CFNotificationCenterRemoveEveryObserver(
                CFNotificationCenterGetDarwinNotifyCenter(),
                Unmanaged.passUnretained(self).toOpaque()
            )
        }
    }
}
