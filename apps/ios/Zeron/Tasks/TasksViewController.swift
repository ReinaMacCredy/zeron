import UIKit

enum TaskGlyph {
    static func asset(_ name: String, side: CGFloat = 20) -> UIImage? {
        guard let asset = UIImage(named: name) else { return nil }
        let rect = CGRect(x: 0, y: 0, width: side, height: side)
        return UIGraphicsImageRenderer(size: rect.size).image { _ in asset.draw(in: rect) }.withRenderingMode(.alwaysTemplate)
    }

    static func kind(_ status: TaskStatus) -> StatusGlyph.Kind {
        switch status {
        case .toDo: .dot(StatusTone.idle)
        case .working: .spinner
        case .needsYou: .dot(StatusTone.input)
        case .done: .check(StatusTone.done)
        }
    }

    static func leading(_ kind: StatusGlyph.Kind, indent: CGFloat = 0) -> UICellAccessory {
        let glyph = StatusGlyph(kind)
        guard indent > 0 else {
            glyph.frame = CGRect(x: 0, y: 0, width: 12, height: 12)
            return .customView(configuration: .init(customView: glyph, placement: .leading(), reservedLayoutWidth: .custom(16)))
        }
        let box = UIView(frame: CGRect(x: 0, y: 0, width: 16 + indent, height: 12))
        glyph.frame = CGRect(x: indent + 2, y: 0, width: 12, height: 12)
        box.addSubview(glyph)
        return .customView(configuration: .init(customView: box, placement: .leading(), reservedLayoutWidth: .custom(16 + indent), maintainsFixedSize: true))
    }

    static func brands(_ harnesses: [String]) -> UICellAccessory? {
        let marks = harnesses.compactMap { BrandMarks.image(for: $0, side: 14) }
        guard !marks.isEmpty else { return nil }
        let stack = UIStackView(arrangedSubviews: marks.map { image in
            let view = UIImageView(image: image)
            view.tintColor = Palette.secondary
            return view
        })
        stack.spacing = 4
        stack.frame = CGRect(x: 0, y: 0, width: CGFloat(marks.count) * 18 - 4, height: 14)
        return .customView(configuration: .init(customView: stack, placement: .trailing()))
    }
}

final class TasksViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!
    private var appToken: AnyObject?
    private let empty = UILabel()
    private let board = KanbanBoardView()
    private static let viewKey = "tasksView"
    private var kanban: Bool { UserDefaults.standard.string(forKey: Self.viewKey) == "kanban" }

    enum Row: Hashable {
        case task(TaskRow)
        case tree(TreeIndexRow)
    }

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
        title = "Tasks"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .always
        let add = UIBarButtonItem(systemItem: .add, primaryAction: UIAction { [weak self] _ in self?.presentNewTask() })
        add.accessibilityIdentifier = "tasks-add"
        navigationItem.rightBarButtonItem = add
        let views = UISegmentedControl(items: tasksViewLabels())
        views.selectedSegmentIndex = kanban ? 1 : 0
        views.accessibilityIdentifier = "tasks-view"
        views.addAction(UIAction { [weak self, weak views] _ in
            UserDefaults.standard.set(views?.selectedSegmentIndex == 1 ? "kanban" : "list", forKey: Self.viewKey)
            self?.reload()
        }, for: .valueChanged)
        navigationItem.leftBarButtonItem = UIBarButtonItem(customView: views)
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.accessibilityIdentifier = "tasks-list"
        view.addSubview(collectionView)
        board.frame = view.bounds
        board.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        board.onOpen = { [weak self] row in self?.open(row) }
        board.onDetails = { [weak self] row in self?.presentTask(row.id) }
        board.onMove = { [weak self] row, completed in
            guard let self, let client = self.app.client else { return }
            Task { @MainActor in
                do { try await client.setTaskCompleted(taskId: row.id, completed: completed) } catch { Toast.show(TasksErrors.message(error), in: self.view.window) }
            }
        }
        view.addSubview(board)

        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { cell, _, row in
            var c = UIListContentConfiguration.subtitleCell()
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            c.textProperties.color = Palette.text
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.secondaryTextProperties.color = Palette.secondary
            c.textToSecondaryTextVerticalPadding = 3
            var accessories: [UICellAccessory] = []
            switch row {
            case let .task(task):
                c.text = task.title
                c.secondaryText = Self.taskLine(task)
                accessories.append(TaskGlyph.leading(TaskGlyph.kind(task.status)))
                cell.accessibilityIdentifier = "task-\(task.id)"
            case let .tree(tree):
                c.text = tree.title
                c.secondaryText = [treeLabels().badge, tree.line, tree.agents].compactMap { $0 }.joined(separator: " · ")
                accessories.append(TaskGlyph.leading(TreeOutlineViewController.kind(tree.status)))
                cell.accessibilityIdentifier = "tree-\(tree.root)"
            }
            accessories.append(.disclosureIndicator())
            cell.accessories = accessories
            cell.contentConfiguration = c
            var bg = UIBackgroundConfiguration.listCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionHeader) { [weak self] view, _, path in
            var c = UIListContentConfiguration.header()
            c.text = self?.dataSource.snapshot().sectionIdentifiers[path.section]
            view.contentConfiguration = c
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, row in
            cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        dataSource.supplementaryViewProvider = { cv, _, path in
            cv.dequeueConfiguredReusableSupplementary(using: header, for: path)
        }

        empty.text = "No tasks yet.\nTap + to add one."
        empty.numberOfLines = 0
        empty.textAlignment = .center
        empty.font = Fonts.ui(.sans, 15)
        empty.textColor = Palette.secondary
        empty.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(empty)
        NSLayoutConstraint.activate([
            empty.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            empty.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, constant: -48),
        ])
        reload()
        appToken = app.observe { [weak self] in self?.reload() }
    }

    static func taskLine(_ task: TaskRow) -> String {
        var parts: [String] = [task.project?.name ?? "No project"]
        if let due = task.dueOn, task.status != .done { parts.append(dueLabel(due)) }
        if task.status == .working || task.status == .needsYou, let host = task.host { parts.append(host.name) }
        if task.status == .done, let at = task.completedAtMs {
            parts.append("Done " + relativeTimeLabel(atMs: at, nowMs: Int64(Date().timeIntervalSince1970 * 1000)))
        }
        return parts.joined(separator: " · ")
    }

    static func dueLabel(_ day: String) -> String {
        let parser = DateFormatter()
        parser.calendar = Calendar(identifier: .gregorian)
        parser.dateFormat = "yyyy-MM-dd"
        guard let date = parser.date(from: day) else { return "due \(day)" }
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "due today" }
        if calendar.isDateInYesterday(date) { return "due yesterday" }
        if calendar.isDateInTomorrow(date) { return "due tomorrow" }
        let out = DateFormatter()
        out.setLocalizedDateFormatFromTemplate("MMMd")
        return "due " + out.string(from: date)
    }

    private func reload() {
        let page = app.tasks
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        let trees = app.client?.taskTrees() ?? TaskTrees(running: [], doneToday: [])
        let title = { (status: TaskStatus) in page.board.first { $0.status == status }?.title ?? "" }
        for (status, rows) in [(TaskStatus.needsYou, page.needsYou), (.working, page.working), (.toDo, page.toDo), (.done, page.done)] {
            var items = rows.map(Row.task)
            if status == .working { items += trees.running.map(Row.tree) }
            if status == .done { items += trees.doneToday.map(Row.tree) }
            guard !items.isEmpty else { continue }
            snap.appendSections(["\(title(status)) · \(items.count)"])
            snap.appendItems(items)
        }
        empty.isHidden = snap.numberOfItems > 0
        dataSource.apply(snap, animatingDifferences: view.window != nil)
        collectionView.isHidden = kanban
        board.isHidden = !kanban
        if kanban {
            board.show(page.board)
            empty.isHidden = true
        }
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let row = dataSource.itemIdentifier(for: indexPath) else { return }
        switch row {
        case let .task(task):
            open(task)
        case let .tree(tree):
            navigationController?.pushViewController(TreeOutlineViewController(app: app, rootChatId: tree.root, title: tree.title), animated: true)
        }
    }

    func collectionView(_ collectionView: UICollectionView, contextMenuConfigurationForItemsAt indexPaths: [IndexPath], point: CGPoint) -> UIContextMenuConfiguration? {
        guard let path = indexPaths.first, case let .task(task)? = dataSource.itemIdentifier(for: path), task.leadChatId != nil else { return nil }
        return UIContextMenuConfiguration(actionProvider: { [weak self] _ in
            UIMenu(children: [UIAction(title: "Task details", image: UIImage(systemName: "info.circle")) { _ in self?.presentTask(task.id) }])
        })
    }

    private func open(_ task: TaskRow) {
        guard let chat = task.leadChatId else { return presentTask(task.id) }
        openThread(chat)
    }

    func presentTask(_ taskId: String) {
        let sheet = TaskSheetViewController(app: app, taskId: taskId) { [weak self] chatId in
            self?.dismiss(animated: true) { self?.openThread(chatId) }
        }
        let nav = UINavigationController(rootViewController: sheet)
        if let presentation = nav.sheetPresentationController {
            presentation.detents = [.medium(), .large()]
            presentation.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }

    func presentNewTask() {
        let sheet = NewTaskViewController(app: app) { [weak self] taskId in
            self?.dismiss(animated: true) { self?.presentTask(taskId) }
        }
        let nav = UINavigationController(rootViewController: sheet)
        if let presentation = nav.sheetPresentationController {
            presentation.detents = [.large()]
            presentation.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }

    func openThread(_ chatId: String) {
        TasksViewController.push(thread: chatId, app: app, from: navigationController)
    }

    static func push(thread chatId: String, app: AppModel, from nav: UINavigationController?) {
        guard let session = SessionViewController(app: app, chatId: chatId) else {
            Toast.show("Couldn't open that thread.", in: nav?.view.window)
            return
        }
        nav?.pushViewController(session, animated: true)
    }
}
