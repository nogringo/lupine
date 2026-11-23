import CoreServices
import Foundation

/// Calls `action` once changes under a folder have settled for a second,
/// and when the folder itself moves, until released. Changes the app makes
/// itself, folder sync's included, are not reported.
final class FolderWatcher: @unchecked Sendable {
    private let stream: FSEventStreamRef
    private let queue = DispatchQueue(label: "ovh.uid.lupine.folder-watcher")

    init?(path: String, action: @escaping @Sendable () -> Void) {
        let box = Unmanaged.passRetained(Action(action))
        var context = FSEventStreamContext(
            version: 0,
            info: box.toOpaque(),
            retain: nil,
            release: { info in
                guard let info else { return }
                Unmanaged<Action>.fromOpaque(info).release()
            },
            copyDescription: nil
        )
        let callback: FSEventStreamCallback = { _, info, _, _, _, _ in
            guard let info else { return }
            Unmanaged<Action>.fromOpaque(info).takeUnretainedValue().run()
        }
        guard let stream = FSEventStreamCreate(
            kCFAllocatorDefault,
            callback,
            &context,
            [path] as CFArray,
            FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
            1.0,
            FSEventStreamCreateFlags(
                kFSEventStreamCreateFlagWatchRoot | kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagIgnoreSelf
            )
        ) else {
            box.release()
            return nil
        }
        self.stream = stream
        FSEventStreamSetDispatchQueue(stream, queue)
        FSEventStreamStart(stream)
    }

    deinit {
        FSEventStreamStop(stream)
        FSEventStreamInvalidate(stream)
        FSEventStreamRelease(stream)
    }
}

private final class Action: Sendable {
    let run: @Sendable () -> Void

    init(_ run: @escaping @Sendable () -> Void) {
        self.run = run
    }
}
