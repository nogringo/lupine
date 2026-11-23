import FileProvider
import LupineCore

/// Changes are reported for the whole drive, whatever the container: the
/// working set is the entire drive.
final class FileProviderEnumerator: NSObject, NSFileProviderEnumerator {
    enum Scope {
        /// `nil` for the drive root.
        case folder(String?)
        case workingSet
    }

    private static let defaultPageSize = 200
    private let scope: Scope
    private let drives: DriveAccess

    init(scope: Scope, drives: DriveAccess) {
        self.scope = scope
        self.drives = drives
    }

    func invalidate() {}

    func enumerateItems(for observer: NSFileProviderEnumerationObserver, startingAt page: NSFileProviderPage) {
        do {
            guard let drive = try drives.drive() else {
                observer.finishEnumerating(upTo: nil)
                return
            }
            let limit = UInt32(clamping: max(1, observer.suggestedPageSize ?? Self.defaultPageSize))
            let after = Self.lastItem(of: page)
            let items = switch scope {
            case .folder(let parent): try drive.children(parent: parent, after: after, limit: limit)
            case .workingSet: try drive.allItems(after: after, limit: limit)
            }
            observer.didEnumerate(items.map(FileProviderItem.init))
            let next = items.count == Int(limit) ? items.last.map { NSFileProviderPage(Data($0.id.utf8)) } : nil
            observer.finishEnumerating(upTo: next)
        } catch {
            observer.finishEnumeratingWithError(ProviderError.map(error))
        }
    }

    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        do {
            guard let drive = try drives.drive() else {
                observer.finishEnumeratingChanges(upTo: anchor, moreComing: false)
                return
            }
            let limit = UInt32(clamping: max(1, observer.suggestedBatchSize ?? Self.defaultPageSize))
            let changes = try drive.changes(anchor: anchor.rawValue, limit: limit)
            if !changes.updated.isEmpty {
                observer.didUpdate(changes.updated.map(FileProviderItem.init))
            }
            if !changes.removed.isEmpty {
                observer.didDeleteItems(withIdentifiers: changes.removed.map { NSFileProviderItemIdentifier($0) })
            }
            observer.finishEnumeratingChanges(upTo: NSFileProviderSyncAnchor(changes.anchor), moreComing: changes.more)
        } catch {
            observer.finishEnumeratingWithError(ProviderError.map(error))
        }
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        let anchor = (try? drives.drive()?.anchor()) ?? Data()
        completionHandler(NSFileProviderSyncAnchor(anchor))
    }

    /// Pages are the id of the last item listed. The system's initial
    /// pages are not ids.
    private static func lastItem(of page: NSFileProviderPage) -> String? {
        guard let id = String(data: page.rawValue, encoding: .utf8),
              id.count == 32, id.allSatisfy(\.isHexDigit)
        else {
            return nil
        }
        return id
    }
}
