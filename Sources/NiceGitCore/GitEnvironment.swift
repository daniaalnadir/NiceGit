import Foundation

extension GitClient {
    public static func repositoryEnvironment(_ inherited: [String: String]) -> [String: String] {
        // Parent Git overrides must not redirect this client or change its requested diff context.
        let localKeys: Set<String> = [
            "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG", "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT", "GIT_OBJECT_DIRECTORY", "GIT_DIR", "GIT_WORK_TREE",
            "GIT_IMPLICIT_WORK_TREE", "GIT_GRAFT_FILE", "GIT_INDEX_FILE",
            "GIT_NO_REPLACE_OBJECTS", "GIT_REPLACE_REF_BASE", "GIT_PREFIX",
            "GIT_SHALLOW_FILE", "GIT_COMMON_DIR", "GIT_DIFF_OPTS"
        ]
        return inherited.filter { key, _ in
            !localKeys.contains(key) && !key.hasPrefix("GIT_CONFIG_KEY_") && !key.hasPrefix("GIT_CONFIG_VALUE_")
        }
    }
}
