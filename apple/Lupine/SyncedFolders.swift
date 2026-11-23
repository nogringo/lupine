import AppKit
import FileProvider
import LupineCore
import Observation
import os

/// Folders of this Mac kept in sync with drive folders. The app reaches
/// them through security-scoped bookmarks, watches them with FSEvents and
/// tells the core when they change.
@MainActor
@Observable
final class SyncedFolders {
    struct Row: Identifiable, Equatable {
        let id: Int64
        var path: String
        var state: State

        var name: String {
            URL(filePath: path).lastPathComponent
        }
    }

    enum State: Equatable {
        case waiting
        case syncing
        case synced(failed: Int)
        case failed(String)
    }

    private(set) var rows: [Row] = []
    private(set) var error: String?
    private let sync: FolderSync
    private let domain: NSFileProviderDomain
    /// The pass changed the drive: the session should send, Finder look.
    private let wrote: @MainActor () -> Void
    private let logger = Logger(subsystem: "ovh.uid.lupine", category: "folders")
    private var watchers: [Int64: FolderWatcher] = [:]
    private var accessed: [Int64: URL] = [:]

    init(sync: FolderSync, domain: NSFileProviderDomain, wrote: @escaping @MainActor () -> Void) {
        self.sync = sync
        self.domain = domain
        self.wrote = wrote
    }

    func start() async {
        do {
            for folder in try sync.folders() {
                rows.append(Row(id: folder.id, path: folder.path, state: .waiting))
                await resume(folder)
            }
        } catch {
            fail("Listing synced folders failed", error)
        }
        let sync = sync
        let reporter = FolderReporter(folders: self)
        Task {
            do {
                try await sync.run(observer: reporter)
            } catch {
                self.fail("Folder sync stopped", error)
            }
        }
    }

    /// The drive changed: what it holds may need to reach the folders.
    func changed() {
        sync.changed()
    }

    func add() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Sync"
        panel.message = "Choose a folder to keep in sync with the drive."
        NSApp.activate()
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await add(url) }
    }

    func remove(_ id: Int64) {
        do {
            try sync.removeFolder(id: id)
        } catch {
            fail("Stopping folder sync failed", error)
            return
        }
        watchers[id] = nil
        accessed.removeValue(forKey: id)?.stopAccessingSecurityScopedResource()
        rows.removeAll { $0.id == id }
    }

    private func add(_ url: URL) async {
        error = nil
        do {
            if let drive = try? await NSFileProviderManager(for: domain)?.getUserVisibleURL(for: .rootContainer),
               Self.plainPath(url).hasPrefix(Self.plainPath(drive)) {
                error = "This folder is in the drive already."
                return
            }
            let bookmark = try url.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil)
            let folder = try await sync.addFolder(path: Self.plainPath(url), bookmark: bookmark)
            wrote()
            rows.append(Row(id: folder.id, path: folder.path, state: .waiting))
            await resume(folder)
        } catch {
            fail("Adding the folder failed", error)
        }
    }

    /// Reaches the folder through its bookmark, follows it if it moved, and
    /// watches it.
    private func resume(_ folder: LupineCore.SyncedFolder) async {
        watchers[folder.id] = nil
        accessed.removeValue(forKey: folder.id)?.stopAccessingSecurityScopedResource()
        do {
            guard let bookmark = folder.bookmark else {
                throw CocoaError(.fileReadNoPermission)
            }
            var stale = false
            let url = try URL(resolvingBookmarkData: bookmark, options: .withSecurityScope, relativeTo: nil, bookmarkDataIsStale: &stale)
            guard url.startAccessingSecurityScopedResource() else {
                throw CocoaError(.fileReadNoPermission)
            }
            accessed[folder.id] = url
            let path = Self.plainPath(url)
            if stale || path != folder.path {
                let renewed = stale
                    ? try url.bookmarkData(options: .withSecurityScope, includingResourceValuesForKeys: nil, relativeTo: nil)
                    : bookmark
                try await sync.relocateFolder(id: folder.id, path: path, bookmark: renewed)
                update(folder.id) { $0.path = path }
            }
            let sync = sync
            watchers[folder.id] = FolderWatcher(path: path) { sync.changed() }
        } catch {
            update(folder.id) { $0.state = .failed("Lupine cannot open this folder: \(error.localizedDescription)") }
            logger.error("Resuming a folder failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    fileprivate func report(_ id: Int64, _ state: State) {
        update(id) { $0.state = state }
    }

    fileprivate func reportWaiting() {
        for index in rows.indices {
            rows[index].state = .waiting
        }
    }

    fileprivate func reportWrote() {
        wrote()
    }

    /// The folder may have moved while the app ran: its bookmark finds it.
    fileprivate func recover(_ id: Int64) async {
        guard let folder = try? sync.folders().first(where: { $0.id == id }) else { return }
        let before = folder.path
        await resume(folder)
        if rows.first(where: { $0.id == id })?.path != before {
            sync.changed()
        }
    }

    private func update(_ id: Int64, _ change: (inout Row) -> Void) {
        guard let index = rows.firstIndex(where: { $0.id == id }) else { return }
        change(&rows[index])
    }

    private func fail(_ context: String, _ error: Error) {
        self.error = error.localizedDescription
        logger.error("\(context, privacy: .public): \(error.localizedDescription, privacy: .public)")
    }

    private static func plainPath(_ url: URL) -> String {
        var path = url.standardizedFileURL.path(percentEncoded: false)
        if path.count > 1, path.hasSuffix("/") {
            path.removeLast()
        }
        return path
    }
}

/// Moves what the drive deleted to the Trash, where it can be put back.
final class Trasher: LocalTrash, Sendable {
    func trash(path: String) -> Bool {
        do {
            try FileManager.default.trashItem(at: URL(filePath: path), resultingItemURL: nil)
            return true
        } catch {
            return false
        }
    }
}

/// Called from the core's threads. The main queue keeps reports in order.
private final class FolderReporter: FolderObserver {
    private let folders: SyncedFolders

    init(folders: SyncedFolders) {
        self.folders = folders
    }

    func waiting() {
        onMain { $0.reportWaiting() }
    }

    func syncing(folder: Int64) {
        onMain { $0.report(folder, .syncing) }
    }

    func synced(folder: Int64, wrote: Bool, failed: [String]) {
        onMain {
            $0.report(folder, .synced(failed: failed.count))
            if wrote {
                $0.reportWrote()
            }
        }
    }

    func failed(folder: Int64, reason: String) {
        onMain { folders in
            folders.report(folder, .failed(reason))
            Task { await folders.recover(folder) }
        }
    }

    private func onMain(_ update: @escaping @MainActor @Sendable (SyncedFolders) -> Void) {
        let folders = folders
        DispatchQueue.main.async {
            MainActor.assumeIsolated { update(folders) }
        }
    }
}
