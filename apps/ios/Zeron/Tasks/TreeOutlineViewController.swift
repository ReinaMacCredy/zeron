import UIKit

final class TreeOutlineViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let rootChatId: String
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, Row>!
    private var appToken: AnyObject?
    private var ticker: Timer?
    private var statsTask: Task<Void, Never>?

    struct Row: Hashable {
        var node: TreeNodeRow?
        var label: String?
        var id: String { node?.id ?? "label-\(label ?? "")" }

        static func == (a: Row, b: Row) -> Bool { a.id == b.id }
        func hash(into hasher: inout Hasher) { hasher.combine(id) }
    }

    init(app: AppModel, rootChatId: String, title: String? = nil) {
        self.app = app
        self.rootChatId = rootChatId
        super.init(nibName: nil, bundle: nil)
        self.title = title
        navigationItem.largeTitleDisplayMode = .never
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit {
        ticker?.invalidate()
        statsTask?.cancel()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.accessibilityIdentifier = "tree-outline"
        view.addSubview(collectionView)

        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { [weak self] cell, _, row in
            guard let self else { return }
            var bg = UIBackgroundConfiguration.listCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
            guard let node = row.node else {
                var c = UIListContentConfiguration.cell()
                c.text = row.label
                c.textProperties.font = Fonts.ui(.sansMedium, 12)
                c.textProperties.color = Palette.secondary
                cell.contentConfiguration = c
                cell.accessories = []
                cell.indentationLevel = 0
                cell.accessibilityIdentifier = "tree-group-\(row.label ?? "")"
                return
            }
            var c = UIListContentConfiguration.subtitleCell()
            c.textProperties.font = Fonts.ui(node.depth == 0 ? .sansSemibold : .sansMedium, 15)
            c.textProperties.color = node.muted ? Palette.secondary : Palette.text
            c.textProperties.numberOfLines = 2
            c.secondaryTextProperties.font = Fonts.ui(.sans, 12.5)
            c.secondaryTextProperties.color = Palette.secondary
            c.textToSecondaryTextVerticalPadding = 2
            switch node.kind {
            case .root: c.text = self.title ?? node.title
            case let .step(n): c.text = "\(n). \(node.title)"
            case .agent, .native: c.text = node.title
            }
            c.secondaryText = [node.word, node.time, node.tokens].compactMap { $0 }.joined(separator: " · ")
            var accessories = [TaskGlyph.leading(Self.kind(node.status))]
            if let brand = TaskGlyph.brands([node.brand].compactMap { $0 }) { accessories.append(brand) }
            if node.chatId != nil { accessories.append(.disclosureIndicator()) }
            cell.accessories = accessories
            cell.contentConfiguration = c
            cell.indentationLevel = Int(node.depth)
            cell.indentationWidth = 14
            cell.accessibilityIdentifier = "tree-node-\(node.id)"
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, row in
            cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        reload()
        appToken = app.observe { [weak self] in self?.reload() }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        statsTask?.cancel()
        statsTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self, let client = self.app.client else { return }
                let changed = (try? await client.refreshOutlineStats(rootChatId: self.rootChatId)) ?? false
                if changed { self.reload() }
                try? await Task.sleep(for: .seconds(10))
            }
        }
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        statsTask?.cancel()
        statsTask = nil
    }

    static func kind(_ status: TreeStatus) -> StatusGlyph.Kind {
        switch status {
        case .running: .spinner
        case .waiting: .dot(Palette.warning)
        case .done: .check(StatusTone.done)
        case .failed: .dot(StatusTone.failed)
        case .queued, .stopped: .dot(StatusTone.idle)
        }
    }

    private func reload() {
        guard let client = app.client else { return }
        let nodes = client.taskOutline(rootChatId: rootChatId)
        var rows: [Row] = []
        for node in nodes {
            if let group = node.group { rows.append(Row(label: group)) }
            rows.append(Row(node: node))
        }
        var snap = NSDiffableDataSourceSnapshot<Int, Row>()
        snap.appendSections([0])
        snap.appendItems(rows)
        snap.reconfigureItems(rows.filter { row in dataSource.snapshot().itemIdentifiers.contains(row) })
        dataSource.apply(snap, animatingDifferences: view.window != nil)
        let live = nodes.contains { $0.status == .running || $0.status == .waiting }
        if live, ticker == nil {
            ticker = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.reload() }
        } else if !live {
            ticker?.invalidate()
            ticker = nil
        }
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        dataSource.itemIdentifier(for: indexPath)?.node?.chatId != nil
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let chatId = dataSource.itemIdentifier(for: indexPath)?.node?.chatId else { return }
        TasksViewController.push(thread: chatId, app: app, from: navigationController ?? parent?.navigationController)
    }
}
