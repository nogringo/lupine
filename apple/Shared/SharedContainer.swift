import Foundation

/// What the app and the File Provider extension share: each drive's
/// database in the App Group container, and drive secrets in the keychain.
enum SharedContainer {
    static let domainIdentifier = "drive"
    static let appGroup = infoString("LupineAppGroup")
    static let keychainGroup = infoString("LupineKeychainGroup")

    static func databaseURL(domain: String = domainIdentifier) throws -> URL {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup) else {
            throw CocoaError(.fileNoSuchFile)
        }
        var drives = container.appending(path: "Drives", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: drives, withIntermediateDirectories: true)
        // A restored copy would reuse the device id it holds.
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try drives.setResourceValues(values)
        return drives.appending(path: "\(domain).sqlite")
    }

    private static func infoString(_ key: String) -> String {
        guard let value = Bundle.main.object(forInfoDictionaryKey: key) as? String else {
            fatalError("\(key) is missing from Info.plist")
        }
        return value
    }
}
