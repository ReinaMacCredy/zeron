import UIKit

final class TaskSheetViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let taskId: String
    private let open: (Destination) -> Void
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!
    private var appToken: AnyObject?
    private var detail: TaskDetail?
    private var thumbs: [String: UIImage] = [:]
    private var loading = Set<String>()

    enum Destination {
        case thread(String)
        case mission(String)
    }

    struct Row: Hashable {
        let id: String
        let title: String
        var subtitle: String?
        var symbol: String?
        var image: UIImage?
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

    private var task: TaskRow? { detail?.row }

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
        collectionView.accessibilityIdentifier = "task-sheet"
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
            if let image = row.image {
                c.image = image
                c.imageProperties.maximumSize = CGSize(width: 44, height: 44)
                c.imageProperties.cornerRadius = 6
            } else if let symbol = row.symbol {
                c.image = UIImage(systemName: symbol) ?? TaskGlyph.asset(symbol)
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
        guard let detail = app.client?.taskDetail(taskId: taskId) else {
            if self.detail != nil { dismiss(animated: true) }
            return
        }
        self.detail = detail
        let task = detail.row
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        snap.appendSections([" task"])
        var about = [Row(id: "title", title: task.title, subtitle: task.notes.isEmpty ? nil : task.notes)]
        about.append(Row(id: "status", title: task.word, subtitle: "Status", symbol: "circle.dashed"))
        if let due = task.dueOn { about.append(Row(id: "due", title: TasksViewController.dueLabel(due), subtitle: "Due", symbol: "calendar")) }
        about.append(Row(id: "project", title: task.project?.name ?? "No project", subtitle: "Project", symbol: "folder"))
        if let host = task.host { about.append(Row(id: "host", title: host.name, subtitle: host.online ? "Runs on" : "Runs on · offline", symbol: "desktopcomputer")) }
        snap.appendItems(about)

        snap.appendSections(["Images · \(detail.images.count)"])
        snap.appendItems(detail.images.enumerated().map { ix, path in
            Row(id: "image:\(path)", title: "Image \(ix + 1)", subtitle: (path as NSString).lastPathComponent, symbol: "photo", image: thumbs[path], action: true)
        })
        loadThumbs(detail.images, device: task.host?.deviceId)
        if detail.images.count < 12 {
            snap.appendItems([Row(id: "add-image", title: "Add images", symbol: "plus", tint: Palette.accent, action: true)])
        }

        if !task.mission || !detail.context.isEmpty {
            snap.appendSections(["Context chats · \(detail.context.count)"])
            snap.appendItems(detail.context.map { chat in
                Row(id: "context:\(chat.chatId)", title: chat.title, symbol: "bubble.left.and.text.bubble.right", action: true)
            })
            if detail.context.count < 5, detail.plan.isEmpty {
                snap.appendItems([Row(id: "add-context", title: "Add a context chat", symbol: "plus", tint: Palette.accent, action: true)])
            }
        }

        var actions: [Row] = []
        if task.mission {
            actions.append(Row(id: "mission", title: missionLabels().openMissionPage, symbol: "glyph-mission", tint: Palette.accent, action: true))
        }
        if let chat = task.leadChatId {
            actions.append(Row(id: "thread:\(chat)", title: "Open thread", symbol: "bubble.left", tint: Palette.accent, action: true))
            if !task.mission, task.status != .done {
                actions.append(Row(id: "unassign", title: "Unassign", symbol: "person.crop.circle.badge.xmark", tint: Palette.text, action: true))
            }
        } else if task.status != .done {
            actions.append(Row(id: "start", title: "Start in a new chat", symbol: "play", tint: Palette.accent, action: true))
            actions.append(Row(id: "assign", title: "Hand to a thread", symbol: "arrowshape.turn.up.right", tint: Palette.accent, action: true))
            actions.append(Row(id: "start-mission", title: missionLabels().planFirst, symbol: "glyph-mission", tint: Palette.accent, action: true))
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

    private var hostId: String? {
        task?.host?.deviceId ?? app.hostOptions.first { $0.online }?.id
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let row = dataSource.itemIdentifier(for: indexPath), let client = app.client else { return }
        let id = taskId
        let source = collectionView.cellForItem(at: indexPath)
        switch row.id {
        case "mission":
            open(.mission(id))
        case let thread where thread.hasPrefix("thread:"):
            open(.thread(String(thread.dropFirst("thread:".count))))
        case "start", "start-mission":
            guard let device = hostId else { return Toast.show("No Mac is online.", in: view.window) }
            let mission = row.id == "start-mission" ? MissionOptions(approvePlan: true, sameAgents: false, planDoc: true) : nil
            let config = app.lastDraft.config
            run { [weak self] in
                let chat = try await client.startTask(taskId: id, deviceId: device, config: config, mission: mission)
                self?.open(mission == nil ? .thread(chat) : .mission(id))
            }
        case "assign":
            ThreadPickerController.present(from: self, app: app, title: "Hand to a thread") { [weak self] chat in
                self?.confirmAssign(chat, from: source)
            }
        case "unassign":
            run { try await client.unassignTask(taskId: id) }
        case "add-image":
            AttachmentPicker(host: self, limit: 12 - (detail?.images.count ?? 0)) { [weak self] picked in self?.addImages(picked) }
                .presentPhotos()
        case let image where image.hasPrefix("image:"):
            let path = String(image.dropFirst("image:".count))
            let sheet = UIAlertController(title: row.title, message: nil, preferredStyle: .actionSheet)
            sheet.addAction(UIAlertAction(title: "Remove", style: .destructive) { [weak self] _ in
                let images = (self?.detail?.images ?? []).filter { $0 != path }
                self?.run { try await client.setTaskImages(taskId: id, images: images) }
            })
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = source
            present(sheet, animated: true)
        case "add-context":
            let device = task?.host?.deviceId
            let taken = Set(detail?.context.map(\.chatId) ?? [])
            ThreadPickerController.present(from: self, app: app, title: "Context chat", include: { chat in
                !taken.contains(chat) && (device == nil || client.sessionRow(chatId: chat)?.deviceId == device)
            }) { [weak self] chat in
                let chats = (self?.detail?.context.map(\.chatId) ?? []) + [chat]
                self?.run { try await client.setTaskContext(taskId: id, chatIds: chats) }
            }
        case let context where context.hasPrefix("context:"):
            let chat = String(context.dropFirst("context:".count))
            let sheet = UIAlertController(title: row.title, message: nil, preferredStyle: .actionSheet)
            sheet.addAction(UIAlertAction(title: "Open thread", style: .default) { [weak self] _ in self?.open(.thread(chat)) })
            if detail?.plan.isEmpty ?? true {
                sheet.addAction(UIAlertAction(title: "Remove", style: .destructive) { [weak self] _ in
                    let chats = (self?.detail?.context.map(\.chatId) ?? []).filter { $0 != chat }
                    self?.run { try await client.setTaskContext(taskId: id, chatIds: chats) }
                })
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = source
            present(sheet, animated: true)
        case "done", "undone":
            run { try await client.setTaskCompleted(taskId: id, completed: row.id == "done") }
        case "delete":
            let alert = UIAlertController(title: "Delete this task?", message: nil, preferredStyle: .actionSheet)
            alert.addAction(UIAlertAction(title: "Delete", style: .destructive) { [weak self] _ in
                self?.run { try await client.deleteTask(taskId: id) }
            })
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            alert.popoverPresentationController?.sourceView = source
            present(alert, animated: true)
        default:
            break
        }
    }

    private func loadThumbs(_ paths: [String], device: String?) {
        guard let client = app.client, let device else { return }
        for path in paths where thumbs[path] == nil && !loading.contains(path) {
            loading.insert(path)
            Task { @MainActor in
                if let data = try? await client.readAttachment(deviceId: device, path: path), let image = UIImage(data: data) {
                    thumbs[path] = image.preparingThumbnail(of: CGSize(width: 88, height: 88)) ?? image
                    reload()
                }
            }
        }
    }

    private func confirmAssign(_ chat: String, from source: UIView?) {
        guard let client = app.client else { return }
        let id = taskId
        let name = app.session(chat)?.title ?? "this thread"
        let sheet = UIAlertController(title: "Hand to \(name)", message: nil, preferredStyle: .actionSheet)
        sheet.addAction(UIAlertAction(title: "Send the task", style: .default) { [weak self] _ in
            self?.run { try await client.assignTask(taskId: id, chatId: chat, send: true) }
        })
        sheet.addAction(UIAlertAction(title: "Link only", style: .default) { [weak self] _ in
            self?.run { try await client.assignTask(taskId: id, chatId: chat, send: false) }
        })
        sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        sheet.popoverPresentationController?.sourceView = source
        present(sheet, animated: true)
    }

    private func addImages(_ picked: [StagedImage]) {
        guard let client = app.client, let device = task?.host?.deviceId ?? hostId, !picked.isEmpty else { return }
        let id = taskId
        let existing = detail?.images ?? []
        run {
            let paths = try await MissionStart.upload(picked, to: device, client: client)
            try await client.setTaskImages(taskId: id, images: Array((existing + paths).prefix(12)))
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
