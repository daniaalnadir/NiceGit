#!/usr/bin/env bash
# Builds a deterministic repository for the NiceGit benchmark (see CONTRIBUTING.md, Performance).
#
#   benchmark-repository.sh <folder> [commits] [branches] [files]
#
# Defaults: 20000 commits, 200 local branches (main included), 3000 files.
#
# Runtime and size, measured on the development Mac (14 cores, git 2.54):
#   generation: about SECONDS_PLACEHOLDER seconds, SIZE_PLACEHOLDER on disk (.git and working files)
#
# The history is one git fast-import stream written by awk, so generating takes seconds rather
# than the minutes that one `git commit` per commit would take. Output is deterministic: a
# Park-Miller generator with a fixed seed chooses every file, and every date and name is fixed.
#
# What it builds:
# - a main line of commits, each changing 1 to 3 files in nested folders;
# - every 50 commits, a merge of a feature branch, which takes 1 or 2 new commits and whose
#   changed files the merge commit repeats (fast-import does not merge trees, so the merge
#   commit carries the feature's versions);
# - the local branches, and refs/remotes/origin/* copies of each (some copies lag behind, so
#   branch tracking shows ahead and behind counts);
# - 100 tags on the main line, half of them annotated;
# - a working tree with 200 modified files, 20 staged changes, and 50 untracked files.
set -euo pipefail

target="${1:?Usage: benchmark-repository.sh <folder> [commits] [branches] [files]}"
commits="${2:-20000}"
branches="${3:-200}"
files="${4:-3000}"

for value in "$commits" "$branches" "$files"; do
    case "$value" in
        '' | *[!0-9]*)
            echo "Commits, branches, and files must be whole numbers." >&2
            exit 2
            ;;
    esac
done
if [ "$commits" -lt 1 ] || [ "$branches" -lt 1 ] || [ "$files" -lt 1 ]; then
    echo "Commits, branches, and files must each be at least 1." >&2
    exit 2
fi
if [ -e "$target" ] && [ -n "$(ls -A "$target" 2>/dev/null)" ]; then
    echo "$target already exists and is not empty." >&2
    exit 2
fi

started=$(date +%s)
mkdir -p "$target"
cd "$target"

git init -q -b main
git config user.name "NiceGit Benchmark"
git config user.email "benchmark@example.invalid"
git config commit.gpgsign false
git config core.autocrlf false

# The stream. Feature branches are numbered 001 to N-1, and main is the one remaining branch.
awk -v commits="$commits" -v branches="$branches" -v files="$files" '
function random(limit) {
    seed = (16807 * seed) % 2147483647
    return seed % limit
}
function path_of(j) {
    return sprintf("module%d/area%d/part%d/file%04d.txt", j % 10, int(j / 10) % 10, int(j / 100) % 5, j)
}
function content_of(p, version,   text, k) {
    text = ""
    for (k = 1; k <= 5; k++) {
        text = text sprintf("line %d of %s, revision %d\n", k, p, version)
    }
    return text
}
function emit_file(p, version,   text) {
    text = content_of(p, version)
    printf "M 100644 inline %s\ndata %d\n%s\n", p, length(text), text
}
function header(ref, m, message, moment) {
    printf "commit %s\nmark :%d\ncommitter %s <%s> %.0f +0000\ndata %d\n%s\n", ref, m, author, email, moment, length(message), message
}
BEGIN {
    seed = 20260101
    author = "NiceGit Benchmark"
    email = "benchmark@example.invalid"
    feature_count = branches - 1
    mark = 0
    moment = 1600000000
    for (j = 0; j < files; j++) {
        revs[j] = 1
    }

    # The first commit creates every file.
    mark++
    head = mark
    main_mark[1] = mark
    header("refs/heads/main", mark, "Create the benchmark project", moment)
    for (j = 0; j < files; j++) {
        emit_file(path_of(j), revs[j])
    }

    for (i = 2; i <= commits; i++) {
        moment += 1800
        mark++
        if (feature_count > 0 && i % 50 == 0) {
            # A feature branch gets one commit of its own, then main merges it.
            feature = int(i / 50) % feature_count + 1
            feature_mark = mark
            feature_base = (feature in feature_tip) ? feature_tip[feature] : head
            touched = 1 + random(2)
            message = sprintf("Work on feature %03d", feature)
            header(sprintf("refs/heads/feature/%03d", feature), feature_mark, message, moment)
            printf "from :%d\n", feature_base
            for (k = 1; k <= touched; k++) {
                j = random(files)
                revs[j]++
                touched_path[k] = path_of(j)
                touched_revision[k] = revs[j]
                emit_file(touched_path[k], touched_revision[k])
            }
            feature_tip[feature] = feature_mark

            mark++
            merge_mark = mark
            moment += 1800
            header("refs/heads/main", merge_mark, sprintf("Merge feature %03d", feature), moment)
            printf "from :%d\nmerge :%d\n", head, feature_mark
            for (k = 1; k <= touched; k++) {
                emit_file(touched_path[k], touched_revision[k])
            }
            head = merge_mark
        } else {
            changes = 1 + random(3)
            message = sprintf("Change %d", i)
            header("refs/heads/main", mark, message, moment)
            printf "from :%d\n", head
            for (k = 1; k <= changes; k++) {
                j = random(files)
                revs[j]++
                emit_file(path_of(j), revs[j])
            }
            head = mark
        }
        main_mark[i] = head
    }

    # Branches with no merge yet start from the main line, and every local branch has a remote copy.
    for (f = 1; f <= feature_count; f++) {
        if (f in feature_tip) {
            tip = feature_tip[f]
        } else {
            tip = head
            printf "reset refs/heads/feature/%03d\nfrom :%d\n", f, tip
        }
        if (f % 3 == 0) {
            lagging = main_mark[commits > 40 ? commits - 40 : 1]
            printf "reset refs/remotes/origin/feature/%03d\nfrom :%d\n", f, lagging
        } else {
            printf "reset refs/remotes/origin/feature/%03d\nfrom :%d\n", f, tip
        }
    }
    printf "reset refs/remotes/origin/main\nfrom :%d\n", main_mark[commits > 20 ? commits - 20 : 1]

    # 100 tags spread along the main line; the even ones are annotated.
    for (t = 1; t <= 100; t++) {
        target_commit = main_mark[1 + int(t * commits / 101)]
        if (t % 2 == 0) {
            printf "tag release-%03d\nfrom :%d\ntagger %s <%s> %.0f +0000\ndata %d\n%s\n", t, target_commit, author, email, moment, length("Release " t), "Release " t
        } else {
            printf "reset refs/tags/release-%03d\nfrom :%d\n", t, target_commit
        }
    }
}' | git fast-import --quiet

# A commit-graph, as `git gc` writes it: fast-import does not, but a repository that has been
# garbage-collected has one, so the benchmark measures the state most repositories are in.
git commit-graph write --reachable --no-progress

# Local branch configuration, so branch tracking has upstreams to count against.
{
    printf '[remote "origin"]\n\turl = https://example.invalid/benchmark.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n'
    printf '[branch "main"]\n\tremote = origin\n\tmerge = refs/heads/main\n'
    for ((f = 1; f <= branches - 1; f++)); do
        name=$(printf 'feature/%03d' "$f")
        printf '[branch "%s"]\n\tremote = origin\n\tmerge = refs/heads/%s\n' "$name" "$name"
    done
} >> .git/config

git reset -q --hard

# The working tree: 200 modified files, 20 staged changes, and 50 untracked files, all chosen
# by position in the sorted file list so they are the same on every run. The lists live in a
# scratch folder outside the repository.
lists=$(mktemp -d)
trap 'rm -rf "$lists"' EXIT
git ls-files > "$lists/files"
awk 'NR % 15 == 1 && count < 200 { print; count++ }' "$lists/files" > "$lists/modified"
awk 'NR % 15 == 8 && count < 20 { print; count++ }' "$lists/files" > "$lists/staged"
while IFS= read -r path; do
    printf 'edited for the benchmark\n' >> "$path"
done < "$lists/modified"
while IFS= read -r path; do
    printf 'staged for the benchmark\n' >> "$path"
    git add -- "$path"
done < "$lists/staged"
mkdir -p scratch/notes
for ((n = 1; n <= 50; n++)); do
    printf 'an untracked note %02d\n' "$n" > "$(printf 'scratch/notes/note-%02d.txt' "$n")"
done

finished=$(date +%s)
echo "Repository: $target"
echo "Commits: $(git rev-list --all --exclude=refs/stash --count)"
echo "Local branches: $(git for-each-ref --format='%(refname)' refs/heads | wc -l | tr -d ' '), remote-tracking branches: $(git for-each-ref --format='%(refname)' refs/remotes | wc -l | tr -d ' '), tags: $(git tag | wc -l | tr -d ' ')"
echo "Tracked files: $(git ls-files | wc -l | tr -d ' ')"
echo "Status: $(git status --porcelain=v1 --untracked-files=all | awk '/^\?\?/ { untracked++ } /^M / { staged++ } /^.M/ { modified++ } END { printf "%d modified, %d staged, %d untracked", modified, staged, untracked }')"
echo "Size: $(du -sh .git | cut -f1) Git data, $(du -sh . | cut -f1) in total"
echo "Generated in $((finished - started)) seconds."
