import NiceGitCore
import SwiftUI

struct FileChangeTree<Row: View>: View {
    let entries: [GitStatusEntry]
    @ViewBuilder var row: (GitStatusEntry) -> Row

    var body: some View {
        ForEach(FileChangeNode.build(entries)) { node in
            FileChangeBranch(node: node, row: row)
        }
    }
}

private struct FileChangeBranch<Row: View>: View {
    let node: FileChangeNode
    @ViewBuilder var row: (GitStatusEntry) -> Row
    @State private var expanded = true

    var body: some View {
        if let entry = node.entry {
            row(entry)
        } else {
            DisclosureGroup(isExpanded: $expanded) {
                ForEach(node.children) { child in
                    FileChangeBranch(node: child, row: row)
                }
            } label: {
                Label(node.name, systemImage: expanded ? "folder.fill" : "folder")
                    .font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }
}

private struct FileChangeNode: Identifiable {
    let id: String
    let name: String
    let entry: GitStatusEntry?
    let children: [FileChangeNode]

    static func build(_ entries: [GitStatusEntry], depth: Int = 0) -> [FileChangeNode] {
        let groups = Dictionary(grouping: entries) { $0.path.split(separator: "/").map(String.init)[depth] }
        return groups.keys.sorted().map { name in
            let group = groups[name] ?? []
            let first = group[0]
            let components = first.path.split(separator: "/").map(String.init)
            if components.count == depth + 1 {
                return FileChangeNode(id: first.path, name: name, entry: first, children: [])
            }
            let children = build(group, depth: depth + 1)
            if children.count == 1, let child = children.first, child.entry == nil {
                return FileChangeNode(id: child.id, name: name + "/" + child.name, entry: nil, children: child.children)
            }
            return FileChangeNode(id: components.prefix(depth + 1).joined(separator: "/"), name: name, entry: nil, children: children)
        }
    }
}
