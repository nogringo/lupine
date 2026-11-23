import FileProvider
import LupineCore

/// Opens the drive on first use, since the app may set it up after the
/// extension started.
final class DriveAccess: @unchecked Sendable {
    private let domain: String
    private let lock = NSLock()
    private var local: LocalDrive?
    private var reader: ContentReader?

    init(domain: String) {
        self.domain = domain
    }

    /// `nil` until the app sets a drive up.
    func drive() throws -> LocalDrive? {
        try lock.withLock { try openDrive() }
    }

    func contentReader() throws -> ContentReader {
        try lock.withLock {
            if let reader {
                return reader
            }
            guard let local = try openDrive(),
                  let secret = try DriveKeychain.secret(drive: local.info().id)
            else {
                throw NSFileProviderError(.notAuthenticated)
            }
            let reader = try openContentReader(drive: local, secret: secret)
            self.reader = reader
            return reader
        }
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
        case .VersionNoLongerAvailable:
            return NSFileProviderError(.versionNoLongerAvailable)
        case .AnchorExpired:
            return NSFileProviderError(.syncAnchorExpired)
        case .Cancelled:
            return CocoaError(.userCancelled)
        case .Unreachable:
            return NSFileProviderError(.serverUnreachable)
        case .Core(let message), .Check(let message):
            return NSError(
                domain: "ovh.uid.lupine",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: message]
            )
        }
    }
}
