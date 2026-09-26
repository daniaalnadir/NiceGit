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

struct FileChangeNode: Identifiable {
    let id: String
    let name: String
    let entry: GitStatusEntry?
    let children: [FileChangeNode]

    static func build(_ entries: [GitStatusEntry], depth: Int = 0) -> [FileChangeNode] {
        let groups = Dictionary(grouping: entries) { $0.path.split(separator: "/").map(String.init)[depth] }
        return groups.keys.sorted().flatMap { name -> [FileChangeNode] in
            let group = groups[name] ?? []
            let path = group[0].path.split(separator: "/").prefix(depth + 1).joined(separator: "/")
            let exact = group.first { $0.path.split(separator: "/").count == depth + 1 }
            let descendants = group.filter { $0.path.split(separator: "/").count > depth + 1 }
            var nodes: [FileChangeNode] = []
            if let exact {
                nodes.append(FileChangeNode(id: "file\0" + exact.path, name: name, entry: exact, children: []))
            }
            if !descendants.isEmpty {
                let children = build(descendants, depth: depth + 1)
                if children.count == 1, let child = children.first, child.entry == nil {
                    nodes.append(FileChangeNode(id: child.id, name: name + "/" + child.name, entry: nil, children: child.children))
                } else {
                    nodes.append(FileChangeNode(id: "folder\0" + path, name: name, entry: nil, children: children))
                }
            }
            return nodes
        }
    }
}
