import FileProvider
import LupineCore
import os
import UniformTypeIdentifiers

/// Enumerates the drive from the local database the app keeps up to date,
/// and fetches content, whole or in part, from Blossom. Finder's changes are
/// uploaded here and written to the local database; the app sends them.
final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension, NSFileProviderPartialContentFetching {
    private let domain: NSFileProviderDomain
    private let drives: DriveAccess
    private let logger = Logger(subsystem: "ovh.uid.lupine.FileProvider", category: "extension")

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        drives = DriveAccess(domain: domain.identifier.rawValue)
        super.init()
        logger.info("Started for \(domain.identifier.rawValue, privacy: .public), core \(coreVersion(), privacy: .public)")
    }

    func invalidate() {}

    func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        if identifier == .rootContainer {
            completionHandler(FileProviderItem.root(), nil)
            return Progress()
        }
        do {
            guard let drive = try drives.drive() else {
                throw NSFileProviderError(.noSuchItem)
            }
            completionHandler(FileProviderItem(try drive.item(id: identifier.rawValue)), nil)
        } catch {
            completionHandler(nil, ProviderError.map(error))
        }
        return Progress()
    }

    func fetchContents(
        for itemIdentifier: NSFileProviderItemIdentifier,
        version requestedVersion: NSFileProviderItemVersion?,
        request: NSFileProviderRequest,
        completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        fetch(itemIdentifier, range: nil, alignment: 1, contentVersion: nil) { url, item, _, error in
            completionHandler(url, item, error)
        }
    }

    func fetchPartialContents(
        for itemIdentifier: NSFileProviderItemIdentifier,
        version requestedVersion: NSFileProviderItemVersion,
        request: NSFileProviderRequest,
        minimalRange requestedRange: NSRange,
        aligningTo alignment: Int,
        options: NSFileProviderFetchContentsOptions = [],
        completionHandler: @escaping (URL?, NSFileProviderItem?, NSRange, NSFileProviderMaterializationFlags, Error?) -> Void
    ) -> Progress {
        let strict = options.contains(.strictVersioning) ? requestedVersion.contentVersion : nil
        return fetch(itemIdentifier, range: requestedRange, alignment: alignment, contentVersion: strict) { url, item, range, error in
            completionHandler(url, item, range, [], error)
        }
    }

    /// Calls `completion` once, with the range written.
    private func fetch(
        _ identifier: NSFileProviderItemIdentifier,
        range: NSRange?,
        alignment: Int,
        contentVersion: Data?,
        completion: @escaping (URL?, NSFileProviderItem?, NSRange, Error?) -> Void
    ) -> Progress {
        let progress = Progress(totalUnitCount: Int64(range?.length ?? 0))
        let url: URL
        let reader: ContentReader
        do {
            reader = try drives.contentReader()
            guard let directory = try NSFileProviderManager(for: domain)?.temporaryDirectoryURL() else {
                throw NSFileProviderError(.providerNotFound)
            }
            url = directory.appending(path: UUID().uuidString)
            if range == nil, let drive = try drives.drive() {
                progress.totalUnitCount = Int64(clamping: try drive.item(id: identifier.rawValue).size)
            }
        } catch {
            completion(nil, nil, NSRange(), ProviderError.map(error))
            return progress
        }

        let request = FetchRequest(
            id: identifier.rawValue,
            path: url.path(percentEncoded: false),
            start: UInt64(range?.location ?? 0),
            end: range.map { UInt64($0.location + $0.length) } ?? .max,
            alignment: UInt64(max(1, alignment)),
            contentVersion: contentVersion
        )
        let cancellation = Cancellation()
        progress.cancellationHandler = { cancellation.cancel() }
        let observer = ProgressObserver(progress)
        let done = Completion(completion)
        let logger = logger
        Task {
            do {
                let fetched = try await reader.fetch(request: request, observer: observer, cancellation: cancellation)
                let written = NSRange(location: Int(fetched.start), length: Int(fetched.end - fetched.start))
                done.call(url, FileProviderItem(fetched.item), written, nil)
            } catch {
                try? FileManager.default.removeItem(at: url)
                logger.error("Fetch failed: \(error.localizedDescription, privacy: .public)")
                done.call(nil, nil, NSRange(), ProviderError.map(error))
            }
        }
        return progress
    }

    func createItem(
        basedOn itemTemplate: NSFileProviderItem,
        fields: NSFileProviderItemFields,
        contents url: URL?,
        options: NSFileProviderCreateItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        let type = itemTemplate.contentType ?? .data
        if Self.staysLocal(itemTemplate.filename, type) {
            completionHandler(nil, [], false, NSFileProviderError(.excludedFromSync))
            return Progress()
        }
        let item = NewItem(
            request: itemTemplate.itemIdentifier.rawValue,
            parent: Self.parent(itemTemplate.parentItemIdentifier),
            name: itemTemplate.filename,
            isFolder: type.conforms(to: .directory),
            mayAlreadyExist: options.contains(.mayAlreadyExist),
            contents: nil,
            created: Self.millis(itemTemplate.creationDate ?? nil),
            modified: Self.millis(itemTemplate.contentModificationDate ?? nil)
        )
        return write(url, completion: completionHandler) { writer, staged, observer, cancellation in
            var item = item
            item.contents = staged?.path(percentEncoded: false)
            return try await writer.createItem(item: item, observer: observer, cancellation: cancellation)
        } failure: { error in
            switch error as? CoreError {
            // The system creates the parent, then tries again.
            case .NoSuchParent?: NSFileProviderError(.noSuchItem)
            // Kept on this Mac only, until renamed.
            case .InvalidName?: NSFileProviderError(.excludedFromSync)
            default: ProviderError.map(error)
            }
        }
    }

    func modifyItem(
        _ item: NSFileProviderItem,
        baseVersion version: NSFileProviderItemVersion,
        changedFields: NSFileProviderItemFields,
        contents newContents: URL?,
        options: NSFileProviderModifyItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        if item.itemIdentifier == .rootContainer {
            completionHandler(FileProviderItem.root(), [], false, nil)
            return Progress()
        }
        let moved = !changedFields.isDisjoint(with: [.filename, .parentItemIdentifier])
        let edit = ItemEdit(
            id: item.itemIdentifier.rawValue,
            isFolder: (item.contentType ?? .data).conforms(to: .directory),
            baseContentVersion: version.contentVersion,
            location: moved ? ItemLocation(parent: Self.parent(item.parentItemIdentifier), name: item.filename) : nil,
            contents: nil,
            created: changedFields.contains(.creationDate) ? Self.millis(item.creationDate ?? nil) : nil,
            modified: changedFields.contains(.contentModificationDate) ? Self.millis(item.contentModificationDate ?? nil) : nil
        )
        // Tags, extended attributes and the like are not kept yet.
        let contents = changedFields.contains(.contents) ? newContents : nil
        return write(contents, completion: completionHandler) { writer, staged, observer, cancellation in
            var edit = edit
            edit.contents = staged?.path(percentEncoded: false)
            return try await writer.modifyItem(edit: edit, observer: observer, cancellation: cancellation)
        } failure: { ProviderError.map($0) }
    }

    func deleteItem(
        identifier: NSFileProviderItemIdentifier,
        baseVersion version: NSFileProviderItemVersion,
        options: NSFileProviderDeleteItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (Error?) -> Void
    ) -> Progress {
        // Only the local database: quick enough to run here.
        do {
            try drives.itemWriter().deleteItem(
                id: identifier.rawValue,
                baseContentVersion: version.contentVersion,
                recursive: options.contains(.recursive)
            )
            OutboxSignal.post()
            // Removals, and a file kept because it changed since.
            NSFileProviderManager(for: domain)?.signalEnumerator(for: .workingSet) { _ in }
            completionHandler(nil)
        } catch {
            logger.error("Delete failed: \(error.localizedDescription, privacy: .public)")
            completionHandler(ProviderError.map(error))
        }
        return Progress()
    }

    /// Uploads `contents` from a clone, so that edits made meanwhile cannot
    /// mix into the version, and calls `completion` once.
    private func write(
        _ contents: URL?,
        completion: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void,
        change: @escaping @Sendable (ItemWriter, URL?, TransferObserver, Cancellation) async throws -> DriveItem?,
        failure: @escaping @Sendable (Error) -> Error
    ) -> Progress {
        let progress = Progress(totalUnitCount: 0)
        let done = Completion(completion)
        let writer: ItemWriter
        let staged: URL?
        do {
            writer = try drives.itemWriter()
            if let contents {
                let size = try contents.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                progress.totalUnitCount = Int64(size)
            }
            staged = contents.map(stage)
        } catch {
            completion(nil, [], false, failure(error))
            return progress
        }
        let cancellation = Cancellation()
        progress.cancellationHandler = { cancellation.cancel() }
        let observer = ProgressObserver(progress)
        let logger = logger
        Task {
            defer {
                if let staged, staged != contents {
                    try? FileManager.default.removeItem(at: staged)
                }
            }
            do {
                let item = try await change(writer, staged, observer, cancellation)
                OutboxSignal.post()
                done.call(item.map(FileProviderItem.init), [], false, nil)
            } catch {
                logger.error("Write failed: \(error.localizedDescription, privacy: .public)")
                done.call(nil, [], false, failure(error))
            }
        }
        return progress
    }

    /// A clone in the domain's temporary directory, or the original where
    /// the volume cannot clone.
    private func stage(_ contents: URL) -> URL {
        guard let directory = try? NSFileProviderManager(for: domain)?.temporaryDirectoryURL() else {
            return contents
        }
        let clone = directory.appending(path: UUID().uuidString)
        let cloned = contents.withUnsafeFileSystemRepresentation { source in
            clone.withUnsafeFileSystemRepresentation { target in
                guard let source, let target else { return false }
                return clonefile(source, target, 0) == 0
            }
        }
        return cloned ? clone : contents
    }

    /// Finder's own files, and links the drive cannot hold.
    private static func staysLocal(_ name: String, _ type: UTType) -> Bool {
        name == ".DS_Store" || type.conforms(to: .symbolicLink) || type.conforms(to: .aliasFile)
    }

    private static func parent(_ identifier: NSFileProviderItemIdentifier) -> String? {
        identifier == .rootContainer ? nil : identifier.rawValue
    }

    private static func millis(_ date: Date?) -> Int64? {
        date.map { Int64(($0.timeIntervalSince1970 * 1000).rounded()) }
    }

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest
    ) throws -> NSFileProviderEnumerator {
        switch containerItemIdentifier {
        case .rootContainer:
            return FileProviderEnumerator(scope: .folder(nil), drives: drives)
        case .workingSet:
            return FileProviderEnumerator(scope: .workingSet, drives: drives)
        case .trashContainer:
            throw NSError(domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError)
        default:
            guard let drive = try drives.drive(),
                  try drive.item(id: containerItemIdentifier.rawValue).isFolder
            else {
                throw NSFileProviderError(.noSuchItem)
            }
            return FileProviderEnumerator(scope: .folder(containerItemIdentifier.rawValue), drives: drives)
        }
    }

}

private final class ProgressObserver: TransferObserver, @unchecked Sendable {
    private let progress: Progress

    init(_ progress: Progress) {
        self.progress = progress
    }

    func progressed(bytes: UInt64) {
        progress.completedUnitCount = Int64(clamping: bytes)
    }
}

/// File Provider completion handlers are not `Sendable`, but may be called
/// from any thread.
private struct Completion<Call>: @unchecked Sendable {
    let call: Call

    init(_ call: Call) {
        self.call = call
    }
}
