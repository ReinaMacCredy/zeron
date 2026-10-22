import UIKit

final class SupervisorAnswersViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let groupKey: String
    private let hostId: String
    private let openThread: (String) -> Void
    private var picks: [String: String] = [:]
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!
    private var appToken: AnyObject?
    private let send = UIButton(configuration: .filled())
    private let summary = UILabel()

    struct Row: Hashable {
        let id: String
        var title: String
        var subtitle: String?
        var picked = false
        var option = false
        var tint: UIColor = Palette.text
    }

    init(app: AppModel, groupKey: String, hostId: String, openThread: @escaping (String) -> Void) {
        self.app = app
        self.groupKey = groupKey
        self.hostId = hostId
        self.openThread = openThread
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    private var entries: [NeedsYouEntry] {
        app.supervisor.needsYou.filter { $0.groupKey == groupKey && $0.host.deviceId == hostId }
    }

    private var decisions: [(entry: NeedsYouEntry, itemId: String, options: [String])] {
        entries.compactMap { entry in
            guard case let .item(itemId, _, _, _, options) = entry.kind, !options.isEmpty else { return nil }
            if case .sending? = entry.send { return nil }
            if case .sent? = entry.send { return nil }
            return (entry, itemId, options)
        }
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        collectionView = UICollectionView(frame: .zero, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.translatesAutoresizingMaskIntoConstraints = false
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.accessibilityIdentifier = "answers"
        view.addSubview(collectionView)

        summary.font = Fonts.ui(.sans, 13)
        summary.textColor = Palette.secondary
        send.configuration?.baseBackgroundColor = Palette.accent
        send.accessibilityIdentifier = "answers-send"
        send.addAction(UIAction { [weak self] _ in self?.sendPicks() }, for: .primaryActionTriggered)
        let footer = UIStackView(arrangedSubviews: [summary, send])
        footer.axis = .vertical
        footer.spacing = 8
        footer.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(footer)
        NSLayoutConstraint.activate([
            collectionView.topAnchor.constraint(equalTo: view.topAnchor),
            collectionView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            collectionView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            collectionView.bottomAnchor.constraint(equalTo: footer.topAnchor, constant: -8),
            footer.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            footer.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
            footer.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -12),
            send.heightAnchor.constraint(equalToConstant: 48),
        ])

        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { cell, _, row in
            var c = UIListContentConfiguration.subtitleCell()
            c.text = row.title
            c.textProperties.font = row.option ? Fonts.ui(.sans, 16) : Fonts.ui(.sansMedium, 16)
            c.textProperties.color = row.tint
            c.textProperties.numberOfLines = 0
            c.secondaryText = row.subtitle
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.secondaryTextProperties.color = row.subtitle?.hasPrefix("Could not send") == true ? Palette.danger : Palette.secondary
            c.secondaryTextProperties.numberOfLines = 0
            if row.option { c.directionalLayoutMargins.leading = 28 }
            cell.contentConfiguration = c
            cell.accessories = row.picked ? [.checkmark(options: .init(tintColor: Palette.accent))] : []
            cell.accessibilityIdentifier = "answers-\(row.id)"
            var bg = UIBackgroundConfiguration.listCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionHeader) { [weak self] view, _, path in
            var c = UIListContentConfiguration.header()
            let name = self?.dataSource.snapshot().sectionIdentifiers[path.section] ?? ""
            c.text = name.hasPrefix(" ") ? nil : name
            view.contentConfiguration = c
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, row in
            cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        dataSource.supplementaryViewProvider = { cv, _, path in
            cv.dequeueConfiguredReusableSupplementary(using: header, for: path)
        }
        for entry in entries {
            if case let .failed(choice, _)? = entry.send, case let .item(itemId, _, _, _, _) = entry.kind {
                picks[itemId] = choice
            }
        }
        reload()
        appToken = app.observe { [weak self] in self?.reload() }
    }

    private func reload() {
        let entries = entries
        guard let first = entries.first else {
            dismiss(animated: true)
            return
        }
        let open = entries.filter { if case .plan = $0.kind { return false } else { return true } }
        title = first.groupTitle
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        snap.appendSections([" about"])
        snap.appendItems([Row(id: "about", title: first.groupTitle, subtitle: "\(first.host.name) · \(open.count) to answer", tint: Palette.text)])
        for entry in open {
            let section = "\(SupervisorViewController.kindWord(entry)) · \(entry.title)"
            snap.appendSections([section])
            switch entry.kind {
            case let .item(itemId, _, raisedByLead, detail, options):
                var about = [raisedByLead ? "Raised by Lead" : "Raised by Supervisor"]
                if !detail.isEmpty { about.append(detail) }
                var rows = [Row(id: "item:\(itemId)", title: about.joined(separator: " · "), subtitle: SupervisorViewController.sendLine(entry), tint: Palette.secondary)]
                let locked = entry.send.map { if case .failed = $0 { return false } else { return true } } ?? false
                if !locked {
                    rows += options.enumerated().map { ix, option in
                        Row(id: "pick:\(itemId):\(ix)", title: option, picked: picks[itemId] == option, option: true)
                    }
                }
                snap.appendItems(rows)
            case let .input(chatId, _, questions):
                let typed = questions.contains { $0.options.isEmpty }
                snap.appendItems([
                    Row(id: "thread:\(chatId):\(entry.key)", title: typed ? "Typed answer · Reply in conversation" : "Answer in the thread", subtitle: questions.first?.question, tint: Palette.accent),
                ])
            case .plan:
                break
            }
        }
        snap.reconfigureItems(snap.itemIdentifiers)
        dataSource.apply(snap, animatingDifferences: view.window != nil)

        let total = decisions.count
        let picked = decisions.filter { picks[$0.itemId] != nil }.count
        summary.text = "\(picked) of \(total) picked"
        let dest = decisions.first?.entry.dest ?? first.dest
        send.configuration?.title = picked > 1 ? "Send \(picked) to \(dest)" : "Send to \(dest)"
        send.isEnabled = picked > 0
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        guard let id = dataSource.itemIdentifier(for: indexPath)?.id else { return false }
        return id.hasPrefix("pick:") || id.hasPrefix("thread:")
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: false)
        guard let id = dataSource.itemIdentifier(for: indexPath)?.id else { return }
        let parts = id.split(separator: ":", maxSplits: 2).map(String.init)
        if parts[0] == "thread" {
            openThread(parts[1])
            return
        }
        guard parts.count == 3, let ix = Int(parts[2]),
              let decision = decisions.first(where: { $0.itemId == parts[1] }), ix < decision.options.count else { return }
        let option = decision.options[ix]
        picks[decision.itemId] = picks[decision.itemId] == option ? nil : option
        reload()
    }

    private func sendPicks() {
        guard let client = app.client else { return }
        let answers = decisions.compactMap { d in picks[d.itemId].map { ItemAnswer(itemId: d.itemId, choice: $0) } }
        guard !answers.isEmpty else { return }
        let host = hostId
        for answer in answers { picks[answer.itemId] = nil }
        Task { @MainActor in
            do {
                try await client.resolveSupervisorItems(deviceId: host, answers: answers)
            } catch {
                for answer in answers { picks[answer.itemId] = answer.choice }
                reload()
            }
        }
    }
}
