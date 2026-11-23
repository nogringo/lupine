import FileProvider
import LupineCore
import UniformTypeIdentifiers

final class FileProviderItem: NSObject, NSFileProviderItem {
    let itemIdentifier: NSFileProviderItemIdentifier
    let parentItemIdentifier: NSFileProviderItemIdentifier
    let filename: String
    let contentType: UTType
    let documentSize: NSNumber?
    let creationDate: Date?
    let contentModificationDate: Date?
    let itemVersion: NSFileProviderItemVersion
    let capabilities: NSFileProviderItemCapabilities

    /// No trash: Finder deletes items for good, after asking.
    private static let shared: NSFileProviderItemCapabilities = [.allowsReading, .allowsRenaming, .allowsReparenting, .allowsDeleting]

    init(_ item: DriveItem) {
        itemIdentifier = NSFileProviderItemIdentifier(item.id)
        parentItemIdentifier = item.parent.map { NSFileProviderItemIdentifier($0) } ?? .rootContainer
        filename = item.name
        let pathExtension = (item.name as NSString).pathExtension
        if item.isFolder {
            // Bundles such as .rtfd documents are folders the system shows as files.
            let package = pathExtension.isEmpty ? nil : UTType(filenameExtension: pathExtension, conformingTo: .directory)
            contentType = package.flatMap { $0.conforms(to: .package) ? $0 : nil } ?? .folder
            documentSize = nil
            capabilities = Self.shared.union([.allowsContentEnumerating, .allowsAddingSubItems])
        } else {
            contentType = pathExtension.isEmpty ? .data : UTType(filenameExtension: pathExtension) ?? .data
            documentSize = NSNumber(value: item.size)
            capabilities = Self.shared.union(.allowsWriting)
        }
        creationDate = item.created.map(Self.date)
        contentModificationDate = item.modified.map(Self.date)
        itemVersion = NSFileProviderItemVersion(
            contentVersion: item.contentVersion,
            metadataVersion: item.metadataVersion
        )
    }

    private init(rootNamed name: String) {
        itemIdentifier = .rootContainer
        parentItemIdentifier = .rootContainer
        filename = name
        contentType = .folder
        documentSize = nil
        creationDate = nil
        contentModificationDate = nil
        itemVersion = NSFileProviderItemVersion(contentVersion: Data(), metadataVersion: Data())
        capabilities = [.allowsReading, .allowsContentEnumerating, .allowsAddingSubItems]
    }

    static func root() -> FileProviderItem {
        FileProviderItem(rootNamed: "Lupine")
    }

    private static func date(_ millis: Int64) -> Date {
        Date(timeIntervalSince1970: Double(millis) / 1000)
    }
}
