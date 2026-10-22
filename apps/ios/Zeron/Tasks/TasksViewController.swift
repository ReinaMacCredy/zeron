import UIKit

enum TaskGlyph {
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

    enum Row: Hashable {
        case supervisor(title: String, subtitle: String)
        case mission(TaskRow)
        case task(TaskRow)
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
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.accessibilityIdentifier = "tasks-list"
        view.addSubview(collectionView)

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
                c.image = UIImage(systemName: "eye")
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

    static func supervisorLine(_ page: SupervisorPage) -> String {
        var parts: [String] = []
        if !page.needsYou.isEmpty { parts.append("\(page.needsYou.count) need\(page.needsYou.count == 1 ? "s" : "") you") }
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
            snap.appendItems([.supervisor(title: "Supervisor", subtitle: Self.supervisorLine(app.supervisor))])
        }
        if !page.missions.isEmpty {
            snap.appendSections(["Missions"])
            snap.appendItems(page.missions.map(Row.mission))
        }
        for (name, rows) in [("Needs you", page.needsYou), ("Working", page.working), ("To do", page.toDo), ("Done", page.done)] where !rows.isEmpty {
            let section = "\(name) · \(rows.count)"
            snap.appendSections([section])
            snap.appendItems(rows.map(Row.task))
        }
        empty.isHidden = snap.numberOfItems > 0
        dataSource.apply(snap, animatingDifferences: view.window != nil)
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

    func presentNewTask() {
        let sheet = NewTaskViewController(app: app) { [weak self] taskId, mission in
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
