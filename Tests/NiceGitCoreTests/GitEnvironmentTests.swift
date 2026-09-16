import Testing
@testable import NiceGitCore

@Test func inheritedEnvironmentCannotRedirectRepositoryOrIndex() {
    let result = GitClient.repositoryEnvironment([
        "GIT_DIR": "/other/.git", "GIT_WORK_TREE": "/other", "GIT_INDEX_FILE": "/other/index",
        "GIT_COMMON_DIR": "/other/common", "GIT_CONFIG_COUNT": "1",
        "GIT_CONFIG_KEY_0": "core.worktree", "GIT_CONFIG_VALUE_0": "/other",
        "GIT_CONFIG_PARAMETERS": "override", "SSH_AUTH_SOCK": "/agent",
        "HOME": "/home/test", "GIT_SSH_COMMAND": "ssh -i key"
    ])
    #expect(result == ["SSH_AUTH_SOCK": "/agent", "HOME": "/home/test", "GIT_SSH_COMMAND": "ssh -i key"])
}
