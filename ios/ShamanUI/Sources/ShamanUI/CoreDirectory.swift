import Foundation

/// The one directory the core keeps everything in.
public enum CoreDirectory {
  /// The directory's name under Application Support, where nothing names another.
  public static let defaultName = "Core"

  /// The directory `name` under `base`, made where it is absent and kept out of every backup. The exclusion is set on
  /// the directory, since the core replaces its files by rename and an exclusion on a file would go with it.
  public static func prepare(named name: String = defaultName, in base: URL = .applicationSupportDirectory) throws
    -> URL
  {
    var directory = base.appending(path: name, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try directory.setResourceValues(values)
    return directory
  }
}
