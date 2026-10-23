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

    static func kind(_ status: StepStatus) -> StatusGlyph.Kind {
        switch status {
        case .planned: .dot(StatusTone.idle)
        case .working: .spinner
        case .needsYou: .dot(Palette.warning)
        case .done: .check(StatusTone.done)
        case .skipped: .dot(Palette.tertiary)
        case .failed: .dot(StatusTone.failed)
        }
    }

    static func word(_ status: StepStatus) -> String {
        switch status {
        case .planned: "Planned"
        case .working: "Working"
        case .needsYou: "Needs you"
        case .done: "Done"
        case .skipped: "Skipped"
        case .failed: "Failed"
        }
    }

    static func leading(_ kind: StatusGlyph.Kind) -> UICellAccessory {
        let glyph = StatusGlyph(kind)
        glyph.frame = CGRect(x: 0, y: 0, width: 12, height: 12)
        return .customView(configuration: .init(customView: glyph, placement: .leading(), reservedLayoutWidth: .custom(16)))
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
        case supervisor(title: String, subtitle: String)
        case mission(TaskRow)
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
        board.onOpen = { [weak self] row in row.mission ? self?.openMission(row.id) : self?.presentTask(row.id) }
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
            case let .supervisor(title, subtitle):
                c.text = title
                c.secondaryText = subtitle
                c.image = TaskGlyph.asset("glyph-supervisor")
                c.imageProperties.tintColor = Palette.accent
                cell.accessibilityIdentifier = "tasks-supervisor"
            case let .mission(task):
                c.text = task.title
                c.secondaryText = Self.missionLine(task)
                accessories.append(TaskGlyph.leading(TaskGlyph.kind(task.status)))
                if let brands = TaskGlyph.brands(task.brands) { accessories.append(brands) }
                cell.accessibilityIdentifier = "mission-\(task.id)"
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

    static func missionLine(_ task: TaskRow) -> String {
        var parts: [String] = []
        if let host = task.host { parts.append(host.name) }
        parts.append(task.missionLabel ?? task.word)
        if task.progressTotal > 0 { parts.append("\(task.progressDone)/\(task.progressTotal) steps") }
        return parts.joined(separator: " · ")
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

    static func supervisorLine(_ page: SupervisorPage, needs: Int) -> String {
        var parts: [String] = []
        if needs > 0 { parts.append("\(needs) need\(needs == 1 ? "s" : "") you") }
        let trees = page.hosts.reduce(0) { $0 + Int($1.trees) }
        let names = page.hosts.map(\.host.name)
        if !names.isEmpty {
            let macs = names.count == 1 ? names[0] : names.dropLast().joined(separator: ", ") + " and " + names.last!
            parts.append("Watching \(trees) tree\(trees == 1 ? "" : "s") on \(macs)")
        }
        if let last = page.hosts.compactMap(\.lastWakeMs).max() {
            parts.append("last wake " + relativeTimeLabel(atMs: last, nowMs: Int64(Date().timeIntervalSince1970 * 1000)))
        }
        return parts.isEmpty ? "No Mac is running a Supervisor" : parts.joined(separator: " · ")
    }

    private func reload() {
        let page = app.tasks
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        if !app.supervisor.hosts.isEmpty {
            snap.appendSections([" "])
            snap.appendItems([.supervisor(title: "Supervisor", subtitle: Self.supervisorLine(app.supervisor, needs: app.needsYou.filter(\.supervised).count))])
        }
        if !page.missions.isEmpty {
            snap.appendSections(["Missions"])
            snap.appendItems(page.missions.map(Row.mission))
        }
        let trees = app.client?.taskTrees() ?? TaskTrees(running: [], doneToday: [])
        let title = { (status: TaskStatus) in page.board.first { $0.status == status }?.title ?? "" }
        for (status, rows) in [(TaskStatus.needsYou, page.needsYou), (.working, page.working), (.toDo, page.toDo), (.done, page.done)] {
            var items = rows.map(Row.task)
            if status == .working { items = trees.running.map(Row.tree) + items }
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
        case .supervisor:
            navigationController?.pushViewController(SupervisorViewController(app: app), animated: true)
        case let .mission(task):
            openMission(task.id)
        case let .task(task):
            presentTask(task.id)
        case let .tree(tree):
            navigationController?.pushViewController(TreeOutlineViewController(app: app, rootChatId: tree.root, title: tree.title), animated: true)
        }
    }

    func openMission(_ taskId: String) {
        navigationController?.pushViewController(MissionControlViewController(app: app, taskId: taskId), animated: true)
    }

    func presentTask(_ taskId: String) {
        let sheet = TaskSheetViewController(app: app, taskId: taskId) { [weak self] destination in
            self?.dismiss(animated: true) {
                switch destination {
                case let .thread(chatId): self?.openThread(chatId)
                case let .mission(taskId): self?.openMission(taskId)
                }
            }
        }
        let nav = UINavigationController(rootViewController: sheet)
        if let presentation = nav.sheetPresentationController {
            presentation.detents = [.medium(), .large()]
            presentation.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard presentedViewController == nil, MissionIntroViewController.due(app: app) else { return }
        let intro = MissionIntroViewController { [weak self] in self?.presentNewTask(planFirst: true) }
        if let sheet = intro.sheetPresentationController {
            sheet.detents = [.medium()]
            sheet.prefersGrabberVisible = true
        }
        present(intro, animated: true)
    }

    func presentNewTask(planFirst: Bool = false) {
        let sheet = NewTaskViewController(app: app, planFirst: planFirst) { [weak self] taskId, mission in
            self?.dismiss(animated: true) {
                mission ? self?.openMission(taskId) : self?.presentTask(taskId)
            }
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
