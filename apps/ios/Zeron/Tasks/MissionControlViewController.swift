import UIKit

final class MissionControlViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let taskId: String
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!
    private var appToken: AnyObject?
    private var countdown: Timer?
    private var detail: TaskDetail?

    struct Row: Hashable {
        let id: String
        var title: String
        var subtitle: String?
        var glyph: Glyph = .none
        var brand: String?
        var tint: UIColor = Palette.text
        var action = false

        enum Glyph: Hashable {
            case none
            case step(StepStatus)
            case task(TaskStatus)
            case symbol(String)
        }
    }

    init(app: AppModel, taskId: String) {
        self.app = app
        self.taskId = taskId
        super.init(nibName: nil, bundle: nil)
        title = "Mission Control"
        navigationItem.largeTitleDisplayMode = .never
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit { countdown?.invalidate() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.accessibilityIdentifier = "mission-control"
        view.addSubview(collectionView)
        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { cell, _, row in
            var c = UIListContentConfiguration.subtitleCell()
            c.text = row.title
            c.textProperties.font = row.action ? Fonts.ui(.sansSemibold, 16) : Fonts.ui(.sansMedium, 16)
            c.textProperties.color = row.tint
            c.textProperties.numberOfLines = 0
            c.secondaryText = row.subtitle
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.secondaryTextProperties.color = Palette.secondary
            c.secondaryTextProperties.numberOfLines = 0
            c.textToSecondaryTextVerticalPadding = 3
            var accessories: [UICellAccessory] = []
            switch row.glyph {
            case .none: break
            case let .step(status): accessories.append(TaskGlyph.leading(TaskGlyph.kind(status)))
            case let .task(status): accessories.append(TaskGlyph.leading(TaskGlyph.kind(status)))
            case let .symbol(name):
                c.image = UIImage(systemName: name)
                c.imageProperties.tintColor = row.tint == Palette.text ? Palette.secondary : row.tint
            }
            if let brand = row.brand, let mark = TaskGlyph.brands([brand]) { accessories.append(mark) }
            if row.id.hasPrefix("step:") { accessories.append(.disclosureIndicator()) }
            cell.accessories = accessories
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "mc-\(row.id)"
            var bg = UIBackgroundConfiguration.listCell()
            bg.backgroundColor = row.action && row.tint == Palette.accent ? Palette.accentSoft : Palette.elevated
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
            navigationController?.popViewController(animated: true)
            return
        }
        self.detail = detail
        let task = detail.row
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        snap.appendSections([" header"])
        var header: [String] = ["Mission"]
        if let host = task.host { header.append(host.name) }
        header.append(task.missionLabel ?? task.word)
        header.append("\(task.progressDone)/\(task.progressTotal) done")
        snap.appendItems([Row(id: "title", title: task.title, subtitle: header.joined(separator: " · "), glyph: .task(task.status))])

        if let approval = detail.approval {
            var rows = [Row(id: "approval", title: approval.line, glyph: .symbol("checkmark.seal"), tint: Palette.text)]
            if approval.canTakeBack {
                rows.append(Row(id: "take-back", title: "Take back", glyph: .symbol("arrow.uturn.backward"), tint: Palette.accent, action: true))
            }
            snap.appendSections([" approval"])
            snap.appendItems(rows)
        }

        if detail.phase == .planReady {
            snap.appendSections(["Plan review"])
            var rows = [Row(
                id: "plan",
                title: "Plan ready · \(detail.steps.count) step\(detail.steps.count == 1 ? "" : "s") · the lead waits for you",
                subtitle: detail.plan.isEmpty ? nil : detail.plan,
                glyph: .symbol("doc.text")
            )]
            rows.append(Row(id: "approve", title: "Approve and run", glyph: .symbol("play.fill"), tint: Palette.accent, action: true))
            snap.appendItems(rows)
        }

        if !detail.steps.isEmpty {
            snap.appendSections(["Steps · \(detail.steps.count)"])
            snap.appendItems(detail.steps.map { step in
                var sub = [TaskGlyph.word(step.status)]
                if let question = step.question { sub.append(question) }
                return Row(
                    id: "step:\(step.id)",
                    title: "\(step.label) · \(step.title)",
                    subtitle: sub.joined(separator: " · "),
                    glyph: .step(step.status),
                    brand: step.brand
                )
            })
        }

        if let lead = task.leadChatId {
            snap.appendSections([" lead"])
            snap.appendItems([Row(id: "lead:\(lead)", title: "Open lead thread", glyph: .symbol("bubble.left"), tint: Palette.accent, action: true)])
        }
        dataSource.apply(snap, animatingDifferences: view.window != nil)

        let held = detail.approval?.held == true
        if held, countdown == nil {
            countdown = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.reload() }
        } else if !held {
            countdown?.invalidate()
            countdown = nil
        }
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        guard let row = dataSource.itemIdentifier(for: indexPath) else { return false }
        return row.action || row.id.hasPrefix("step:")
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let row = dataSource.itemIdentifier(for: indexPath), let client = app.client else { return }
        let id = taskId
        switch row.id {
        case "approve":
            run { try await client.approvePlan(taskId: id) }
        case "take-back":
            run { try await client.takeBackPlanApproval(taskId: id) }
        case let lead where lead.hasPrefix("lead:"):
            TasksViewController.push(thread: String(lead.dropFirst("lead:".count)), app: app, from: navigationController)
        case let step where step.hasPrefix("step:"):
            guard let view = detail?.steps.first(where: { "step:\($0.id)" == step }) else { return }
            stepActions(view, client: client, from: collectionView.cellForItem(at: indexPath))
        default:
            break
        }
    }

    private func stepActions(_ step: StepView, client: CoreClient, from source: UIView?) {
        let sheet = UIAlertController(title: "\(step.label) · \(step.title)", message: step.notes.isEmpty ? nil : step.notes, preferredStyle: .actionSheet)
        if let chat = step.chatId {
            sheet.addAction(UIAlertAction(title: "Open thread", style: .default) { [weak self] _ in
                guard let self else { return }
                TasksViewController.push(thread: chat, app: self.app, from: self.navigationController)
            })
        }
        if step.status == .failed {
            sheet.addAction(UIAlertAction(title: "Retry", style: .default) { [weak self] _ in
                self?.run { try await client.retryStep(stepId: step.id) }
            })
        }
        if step.status == .skipped {
            sheet.addAction(UIAlertAction(title: "Unskip", style: .default) { [weak self] _ in
                self?.run { try await client.setStepSkipped(stepId: step.id, skipped: false) }
            })
        } else if step.status != .done {
            sheet.addAction(UIAlertAction(title: "Skip", style: .destructive) { [weak self] _ in
                self?.run { try await client.setStepSkipped(stepId: step.id, skipped: true) }
            })
        }
        sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        sheet.popoverPresentationController?.sourceView = source
        present(sheet, animated: true)
    }

    private func run(_ action: @escaping () async throws -> Void) {
        Task { @MainActor in
            do { try await action() } catch {
                Toast.show(TasksErrors.message(error), in: self.view.window)
            }
        }
    }
}
