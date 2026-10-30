import SwiftUI
import UIKit

final class LiveActivitySettingsViewController: UIViewController, UICollectionViewDelegate {
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, SettingsRow>!
    private static let footer = "iOS ends a Live Activity after 8 hours; Zeron starts a new one the next time you open it. Live Activities for Zeron can also be turned off in iOS Settings."

    init() {
        super.init(nibName: nil, bundle: nil)
        title = "Live Activity"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        config.footerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.topEdgeEffect.style = .soft
        collectionView.delegate = self
        view.addSubview(collectionView)

        let cell = SettingsRow.registration { [weak self] in
            guard let self else { return nil }
            return SettingsRow.Handlers(toggled: self.toggled, picked: self.picked)
        }
        let preview = UICollectionView.CellRegistration<UICollectionViewListCell, SettingsRow> { cell, _, _ in
            cell.contentConfiguration = UIHostingConfiguration { LiveActivityPreview(prefs: LiveActivityPrefs.current) }
                .margins(.all, 16)
            var bg = UIBackgroundConfiguration.listGroupedCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionHeader) { [weak self] view, _, path in
            var c = UIListContentConfiguration.groupedHeader()
            c.text = self?.dataSource.snapshot().sectionIdentifiers[path.section]
            view.contentConfiguration = c
        }
        let footer = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionFooter) { [weak self] view, _, path in
            var c = UIListContentConfiguration.groupedFooter()
            c.text = path.section == (self?.dataSource.snapshot().numberOfSections ?? 0) - 1 ? Self.footer : nil
            view.contentConfiguration = c
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, row in
            row.id == "preview"
                ? cv.dequeueConfiguredReusableCell(using: preview, for: path, item: row)
                : cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        dataSource.supplementaryViewProvider = { cv, kind, path in
            kind == UICollectionView.elementKindSectionFooter
                ? cv.dequeueConfiguredReusableSupplementary(using: footer, for: path)
                : cv.dequeueConfiguredReusableSupplementary(using: header, for: path)
        }
        reload()
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool { false }

    private func reload() {
        let p = LiveActivityPrefs.current
        let off = !p.enabled
        var s = NSDiffableDataSourceSnapshot<String, SettingsRow>()
        s.appendSections(["Preview"])
        s.appendItems([SettingsRow(id: "preview", title: previewKey(p), symbol: "")])
        s.appendSections(["Live Activity"])
        s.appendItems([
            SettingsRow(id: "enabled", title: "Live Activity", subtitle: "Running sessions on the Dynamic Island and Lock Screen", symbol: "platter.filled.top.iphone", accessory: .toggle(p.enabled)),
        ])
        s.appendSections(["Show"])
        s.appendItems([
            SettingsRow(id: "mode", title: "Layout", symbol: "rectangle.stack", accessory: .segmented(["Grouped", "Per session"], LiveActivityPrefs.Mode.allCases.firstIndex(of: p.mode) ?? 0), disabled: off),
            SettingsRow(id: "which", title: "Sessions", symbol: "line.3.horizontal.decrease", accessory: .segmented(["All", "Pinned", "Asks"], LiveActivityPrefs.Which.allCases.firstIndex(of: p.which) ?? 0), disabled: off),
            SettingsRow(id: "trailing", title: "Island right", symbol: "capsule", accessory: .segmented(["Count", "Clock", "State"], SessionActivityTrailing.allCases.firstIndex(of: p.trailing) ?? 0), disabled: off),
            SettingsRow(id: "showStep", title: "Current step", subtitle: "What each session is doing now", symbol: "text.line.first.and.arrowtriangle.forward", accessory: .toggle(p.showStep), disabled: off),
        ])
        s.appendSections(["Lock Screen and alerts"])
        s.appendItems([
            SettingsRow(id: "lockHide", title: "Hide titles when locked", subtitle: "Shows project and branch instead", symbol: "lock", accessory: .toggle(p.lockHide), disabled: off),
            SettingsRow(id: "alertAsk", title: "Expand when a session needs you", symbol: "questionmark.bubble", accessory: .toggle(p.alertAsk), disabled: off),
        ])
        s.appendSections(["When done"])
        s.appendItems([
            SettingsRow(id: "endAfter", title: "Keep showing", symbol: "timer", accessory: .segmented(["No", "15 min", "1 hour"], LiveActivityPrefs.EndAfter.allCases.firstIndex(of: p.endAfter) ?? 0), disabled: off),
        ])
        dataSource.apply(s, animatingDifferences: false)
    }

    private func previewKey(_ p: LiveActivityPrefs) -> String {
        "\(p.enabled)-\(p.mode)-\(p.trailing)-\(p.showStep)"
    }

    private func toggled(_ id: String, _ on: Bool) {
        var p = LiveActivityPrefs.current
        switch id {
        case "enabled": p.enabled = on
        case "showStep": p.showStep = on
        case "lockHide": p.lockHide = on
        case "alertAsk": p.alertAsk = on
        default: return
        }
        LiveActivityPrefs.current = p
        reload()
    }

    private func picked(_ id: String, _ index: Int) {
        var p = LiveActivityPrefs.current
        switch id {
        case "mode": p.mode = LiveActivityPrefs.Mode.allCases[index]
        case "which": p.which = LiveActivityPrefs.Which.allCases[index]
        case "trailing": p.trailing = SessionActivityTrailing.allCases[index]
        case "endAfter": p.endAfter = LiveActivityPrefs.EndAfter.allCases[index]
        default: return
        }
        LiveActivityPrefs.current = p
        reload()
    }
}

private struct LiveActivityPreview: View {
    let prefs: LiveActivityPrefs

    var body: some View {
        let attributes = prefs.attributes(chatId: prefs.mode == .grouped ? nil : "preview")
        let state = sample
        VStack(spacing: 12) {
            SessionIslandCompact(state: state, attributes: attributes)
                .background(Color.black, in: Capsule())
            SessionIslandExpanded(state: state, attributes: attributes)
                .background(Color.black, in: RoundedRectangle(cornerRadius: 36, style: .continuous))
        }
        .foregroundStyle(.white)
        .environment(\.colorScheme, .dark)
        .frame(maxWidth: .infinity)
        .opacity(prefs.enabled ? 1 : 0.4)
        .allowsHitTesting(false)
    }

    private var sample: SessionActivityAttributes.ContentState {
        let now = Date().timeIntervalSince1970
        let rows = [
            SessionActivityRow(id: "b", title: "Live Activity", project: "zeron", branch: "ios-island", state: .asking, since: now - 95, step: "Allow edit to edge/src?"),
            SessionActivityRow(id: "preview", title: "Fix sync retry", project: "zeron", branch: "main", state: .working, since: now - 312, step: "Running the edge tests"),
            SessionActivityRow(id: "c", title: "Release notes", project: "site", branch: nil, state: .done, since: nil, step: nil),
        ]
        if prefs.mode == .session {
            return .init(rows: [rows[1]], running: 1, asking: 0)
        }
        return .init(rows: rows, running: 1, asking: 1)
    }
}

private struct SessionIslandCompact: View {
    let state: SessionActivityAttributes.ContentState
    let attributes: SessionActivityAttributes

    var body: some View {
        HStack(spacing: 0) {
            SessionCompactLeading(state: state)
            Spacer(minLength: 126)
            SessionCompactTrailing(state: state, attributes: attributes)
        }
        .padding(.horizontal, 12)
        .frame(width: 218, height: 37)
    }
}

private struct SessionIslandExpanded: View {
    let state: SessionActivityAttributes.ContentState
    let attributes: SessionActivityAttributes

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 0) {
                SessionExpandedLeading(state: state, attributes: attributes)
                Spacer(minLength: 12)
                SessionExpandedTrailing(state: state, attributes: attributes)
            }
            .frame(height: 26)
            SessionExpandedBody(state: state, attributes: attributes)
        }
        .padding(EdgeInsets(top: 8, leading: 22, bottom: 18, trailing: 22))
        .frame(maxWidth: .infinity)
    }
}
