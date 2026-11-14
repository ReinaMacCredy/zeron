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
        var node: TreeNodeRow
        var id: String { node.id }

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
            let node = row.node
            var c = UIListContentConfiguration.subtitleCell()
            c.textProperties.font = Fonts.ui(node.depth == 0 ? .sansSemibold : .sansMedium, 15)
            c.textProperties.color = node.muted ? Palette.secondary : Palette.text
            c.textProperties.numberOfLines = 2
            c.secondaryTextProperties.font = Fonts.ui(.sans, 12.5)
            c.secondaryTextProperties.color = Palette.secondary
            c.textToSecondaryTextVerticalPadding = 2
            c.text = node.kind == .root ? self.title ?? node.title : node.title
            c.secondaryText = [node.word, node.time, node.tokens].compactMap { $0 }.joined(separator: " · ")
            var accessories = [TaskGlyph.leading(Self.kind(node.status), indent: CGFloat(node.depth) * Self.indent)]
            if let brand = TaskGlyph.brands([node.brand].compactMap { $0 }) { accessories.append(brand) }
            if node.chatId != nil { accessories.append(.disclosureIndicator()) }
            cell.accessories = accessories
            cell.contentConfiguration = c
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

    private static let indent: CGFloat = 14

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
        try? client.watchOutlineDocs(rootChatId: rootChatId)
        let nodes = client.taskOutline(rootChatId: rootChatId)
        let rows = nodes.map { Row(node: $0) }
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
        dataSource.itemIdentifier(for: indexPath)?.node.chatId != nil
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let chatId = dataSource.itemIdentifier(for: indexPath)?.node.chatId else { return }
        TasksViewController.push(thread: chatId, app: app, from: navigationController ?? parent?.navigationController)
    }
}
