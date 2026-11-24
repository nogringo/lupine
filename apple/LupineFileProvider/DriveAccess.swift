import FileProvider
import LupineCore

/// Opens the drive on first use, since the app may set it up after the
/// extension started. The reader and the writer follow the drive's servers,
/// which the app may change.
final class DriveAccess: @unchecked Sendable {
    private let domain: String
    private let lock = NSLock()
    private var local: LocalDrive?
    private var reader: (ContentReader, servers: [[String]])?
    private var writer: (ItemWriter, servers: [[String]])?

    init(domain: String) {
        self.domain = domain
    }

    /// `nil` until the app sets a drive up.
    func drive() throws -> LocalDrive? {
        try lock.withLock { try openDrive() }
    }

    func contentReader() throws -> ContentReader {
        try lock.withLock {
            let (local, secret, servers) = try unlocked()
            if let reader, reader.servers == servers {
                return reader.0
            }
            let opened = try openContentReader(drive: local, secret: secret)
            reader = (opened, servers)
            return opened
        }
    }

    func itemWriter() throws -> ItemWriter {
        try lock.withLock {
            let (local, secret, servers) = try unlocked()
            if let writer, writer.servers == servers {
                return writer.0
            }
            let opened = try openItemWriter(drive: local, secret: secret)
            writer = (opened, servers)
            return opened
        }
    }

    private func unlocked() throws -> (LocalDrive, String, servers: [[String]]) {
        guard let local = try openDrive() else {
            throw NSFileProviderError(.notAuthenticated)
        }
        let info = try local.info()
        guard let secret = try DriveKeychain.secret(drive: info.id) else {
            throw NSFileProviderError(.notAuthenticated)
        }
        return (local, secret, [info.servers, info.leaving])
    }

    private func openDrive() throws -> LocalDrive? {
        if let local {
            return local
        }
        let path = try SharedContainer.databaseURL(domain: domain).path(percentEncoded: false)
        local = try openLocalDrive(path: path)
        return local
    }
}

enum ProviderError {
    static func map(_ error: Error) -> Error {
        guard let error = error as? CoreError else {
            return error
        }
        switch error {
        case .NoSuchItem:
            return NSFileProviderError(.noSuchItem)
        case .DirectoryNotEmpty:
            return NSFileProviderError(.directoryNotEmpty)
        case .VersionNoLongerAvailable:
            return NSFileProviderError(.versionNoLongerAvailable)
        case .AnchorExpired:
            return NSFileProviderError(.syncAnchorExpired)
        case .Cancelled:
            return CocoaError(.userCancelled)
        case .Unreachable:
            return NSFileProviderError(.serverUnreachable)
        case .NoSuchParent(let message), .InvalidName(let message), .Core(let message), .Check(let message):
            return NSError(
                domain: "ovh.uid.lupine",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: message]
            )
        }
    }
}
