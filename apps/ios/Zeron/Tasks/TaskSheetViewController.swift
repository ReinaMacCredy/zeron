import UIKit

final class TaskSheetViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let taskId: String
    private let open: (Destination) -> Void
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!
    private var appToken: AnyObject?

    enum Destination {
        case thread(String)
        case mission(String)
    }

    struct Row: Hashable {
        let id: String
        let title: String
        var subtitle: String?
        var symbol: String?
        var tint: UIColor = Palette.text
        var action = false
    }

    init(app: AppModel, taskId: String, open: @escaping (Destination) -> Void) {
        self.app = app
        self.taskId = taskId
        self.open = open
        super.init(nibName: nil, bundle: nil)
        title = "Task"
    }

    required init?(coder: NSCoder) { fatalError() }

    private var task: TaskRow? {
        let page = app.tasks
        return (page.missions + page.needsYou + page.working + page.toDo + page.done).first { $0.id == taskId }
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
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        view.addSubview(collectionView)
        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { cell, _, row in
            var c = row.action ? UIListContentConfiguration.cell() : UIListContentConfiguration.subtitleCell()
            c.text = row.title
            c.textProperties.font = row.action ? Fonts.ui(.sansMedium, 16) : Fonts.ui(.sans, 16)
            c.textProperties.color = row.tint
            c.textProperties.numberOfLines = 0
            c.secondaryText = row.subtitle
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.secondaryTextProperties.color = Palette.secondary
            if let symbol = row.symbol {
                c.image = UIImage(systemName: symbol)
                c.imageProperties.tintColor = row.tint == Palette.text ? Palette.secondary : row.tint
            }
            cell.contentConfiguration = c
            cell.accessories = []
            cell.accessibilityIdentifier = "task-sheet-\(row.id)"
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
        reload()
        appToken = app.observe { [weak self] in self?.reload() }
    }

    private func reload() {
        guard let task else {
            dismiss(animated: true)
            return
        }
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        snap.appendSections([" task"])
        var about = [Row(id: "title", title: task.title, subtitle: task.notes.isEmpty ? nil : task.notes)]
        about.append(Row(id: "status", title: task.word, subtitle: "Status", symbol: "circle.dashed"))
        if let due = task.dueOn { about.append(Row(id: "due", title: TasksViewController.dueLabel(due), subtitle: "Due", symbol: "calendar")) }
        about.append(Row(id: "project", title: task.project?.name ?? "No project", subtitle: "Project", symbol: "folder"))
        if let host = task.host { about.append(Row(id: "host", title: host.name, subtitle: host.online ? "Runs on" : "Runs on · offline", symbol: "desktopcomputer")) }
        snap.appendItems(about)
        var actions: [Row] = []
        if task.mission {
            actions.append(Row(id: "mission", title: "Open Mission Control", symbol: "list.bullet.rectangle", tint: Palette.accent, action: true))
        }
        if let chat = task.leadChatId {
            actions.append(Row(id: "thread:\(chat)", title: "Open thread", symbol: "bubble.left", tint: Palette.accent, action: true))
        }
        actions.append(task.status == .done
            ? Row(id: "undone", title: "Mark as not done", symbol: "arrow.uturn.backward", tint: Palette.text, action: true)
            : Row(id: "done", title: "Mark as done", symbol: "checkmark.circle", tint: Palette.text, action: true))
        actions.append(Row(id: "delete", title: "Delete", symbol: "trash", tint: Palette.danger, action: true))
        snap.appendSections([" actions"])
        snap.appendItems(actions)
        dataSource.apply(snap, animatingDifferences: false)
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        dataSource.itemIdentifier(for: indexPath)?.action ?? false
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let row = dataSource.itemIdentifier(for: indexPath), let client = app.client else { return }
        let id = taskId
        switch row.id {
        case "mission":
            open(.mission(id))
        case let thread where thread.hasPrefix("thread:"):
            open(.thread(String(thread.dropFirst("thread:".count))))
        case "done", "undone":
            run { try await client.setTaskCompleted(taskId: id, completed: row.id == "done") }
        case "delete":
            let alert = UIAlertController(title: "Delete this task?", message: nil, preferredStyle: .actionSheet)
            alert.addAction(UIAlertAction(title: "Delete", style: .destructive) { [weak self] _ in
                self?.run { try await client.deleteTask(taskId: id) }
            })
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            alert.popoverPresentationController?.sourceView = collectionView.cellForItem(at: indexPath)
            present(alert, animated: true)
        default:
            break
        }
    }

    private func run(_ action: @escaping () async throws -> Void) {
        Task { @MainActor in
            do { try await action() } catch {
                Toast.show(TasksErrors.message(error), in: self.view.window)
            }
        }
    }
}

enum TasksErrors {
    static func message(_ error: Error) -> String {
        switch error as? CoreError {
        case .HostUnavailable?: return "That Mac is offline."
        case .Unsupported?: return "That Mac's Zeron is too old for this."
        default: return "That didn't go through. Try again."
        }
    }
}
