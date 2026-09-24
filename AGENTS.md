# NiceGit working notes

- A branch switch must preserve staged, unstaged, and untracked changes. Keep the saved stash visible, and restore the original checkout if the switch fails.
- When reducing repository refresh calls, retain detached-HEAD and linked-worktree behavior in integration tests.
- Check busy state before changing repository selection or editor state. Base post-operation notices on the Git result rather than a possibly stale snapshot.
- Git metadata paths can contain newlines; use a single absolute Git directory path and remove only Git's final line terminator. Validate that a selected stash is still listed before applying it.
- For actions on a selected branch, compare its current ref tip with the tip shown when selected before renaming, deleting, pushing, or changing upstream settings.
- Before switching branches, confirm that stash push created a new stash and cleared the working tree; a superproject stash does not save dirty submodule files.
