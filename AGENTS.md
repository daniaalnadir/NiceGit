# NiceGit working notes

- A branch switch must preserve staged, unstaged, and untracked changes. Keep the saved stash visible, and restore the original checkout if the switch fails.
- When reducing repository refresh calls, retain detached-HEAD and linked-worktree behavior in integration tests.
- Check busy state before changing repository selection or editor state. Base post-operation notices on the Git result rather than a possibly stale snapshot.
- Git metadata paths can contain newlines; use a single absolute Git directory path and remove only Git's final line terminator. Validate that a selected stash is still listed before applying it.
- For actions on a selected branch, compare its current ref tip with the tip shown when selected before renaming, deleting, pushing, or changing upstream settings.
- Before switching branches, confirm that stash push created a new stash and cleared the working tree; a superproject stash does not save dirty submodule files.
- Discard must handle staged and unstaged changes together, restore both paths of a rename, and remove selected untracked files without touching other paths.
- Use `git switch --no-overwrite-ignore`: Git otherwise overwrites ignored local files when a target branch tracks the same path.
- Set literal pathspecs for every Git command receiving a selected file path, including `git clean`; glob characters in a filename can otherwise select and delete other files.
- Delete selected tags with the exact ref object ID captured when selected, using an atomic `update-ref -d` check so a replaced tag survives stale confirmation.
- Treat `git stash push` as incomplete until it creates a new stash and leaves only intentionally excluded untracked files; submodule edits can remain after Git reports success.
- Push and publish only the selected current branch with an explicit refspec, disabling mirror and automatic tag following; plain `git push` can send other branches or tags under user Git settings.
