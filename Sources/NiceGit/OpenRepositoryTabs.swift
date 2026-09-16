import SwiftUI

struct OpenRepositoryTabs: View {
    @EnvironmentObject private var model: AppModel
    @Binding var isCollapsed: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button { isCollapsed.toggle() } label: {
                    Image(systemName: "sidebar.left").frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .help(isCollapsed ? "Show repositories" : "Hide repositories")
                .accessibilityLabel(isCollapsed ? "Show repositories" : "Hide repositories")
                if !isCollapsed {
                    Text("Repositories").font(.system(size: 13, weight: .semibold))
                    Spacer(minLength: 0)
                }
            }.padding(.horizontal, 8).frame(height: 52)
            if !isCollapsed {
            HStack(spacing: 8) {
                Text("OPEN TABS").font(.system(size: 11, weight: .semibold))
                Text(model.openRepositories.count.formatted()).font(.caption.monospaced()).foregroundStyle(.secondary)
                Spacer()
                Button { model.openRepository() } label: { Image(systemName: "plus") }
                    .buttonStyle(.plain).help("Open another repository")
            }.padding(.horizontal, 16).padding(.bottom, 8)
            if model.openRepositories.isEmpty {
                Text("No open repositories").font(.caption).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 16).padding(.bottom, 12)
            } else {
                ScrollView {
                    VStack(spacing: 2) {
                        ForEach(model.openRepositories) { repository in
                            let active = model.snapshot?.rootPath == repository.path
                            HStack(spacing: 6) {
                                Button {
                                    if !active { model.loadRepository(at: URL(fileURLWithPath: repository.path)) }
                                } label: {
                                    HStack(spacing: 8) {
                                        Image(systemName: active ? "folder.fill" : "folder").foregroundStyle(active ? AppPalette.signal : .secondary)
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(repository.name).font(.system(size: 12, weight: active ? .semibold : .regular)).lineLimit(1)
                                            Text(repository.path).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                                        }
                                        Spacer(minLength: 0)
                                    }.contentShape(Rectangle())
                                }.buttonStyle(.plain).help(repository.path)
                                    .accessibilityLabel("\(repository.name) tab\(active ? ", active" : "")")
                                Button { model.closeRepository(path: repository.path) } label: {
                                    Image(systemName: "xmark").font(.system(size: 10)).frame(width: 22, height: 28).contentShape(Rectangle())
                                }.buttonStyle(.plain).help("Close \(repository.name) tab")
                                    .accessibilityLabel("Close \(repository.name) tab")
                            }.padding(.horizontal, 8).frame(height: 44)
                                .background(active ? AppPalette.selection : Color.clear)
                                .clipShape(RoundedRectangle(cornerRadius: 6))
                        }
                    }.padding(.horizontal, 8)
                }.frame(maxHeight: .infinity)
                    .padding(.bottom, 10)
            }
            }
            Spacer(minLength: 0)
        }
        .frame(width: isCollapsed ? 44 : 220)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(AppPalette.sidebar)
    }
}
