import UIKit

final class MissionControlViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let taskId: String
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!
    private var appToken: AnyObject?
    private var countdown: Timer?
    private var detail: TaskDetail?
    private let plan = PlanMarkdownView()
    private var editingPlan = false
    private var draft = Draft()
    private var estimate: PlanEstimate?
    private var estimateKey: String?
    private var outline: TreeOutlineViewController?

    struct Row: Hashable {
        let id: String
        var title: String
        var subtitle: String?
        var glyph: Glyph = .none
        var brand: String?
        var tint: UIColor = Palette.text
        var action = false
        var editable = false

        enum Glyph: Hashable {
            case none
            case step(StepStatus)
            case task(TaskStatus)
            case symbol(String)
        }
    }

    struct StepText: Equatable {
        var title: String
        var notes: String
    }

    struct Draft {
        var edited: [String: StepText] = [:]
        var added: [StepText] = []
        var removed: Set<String> = []
        var order: [String]?

        var count: Int { edited.count + added.count + removed.count + (order == nil ? 0 : 1) }

        func shown(_ steps: [StepView]) -> [StepView] {
            var ordered = steps
            if let order {
                ordered = order.compactMap { id in steps.first { $0.id == id } } + steps.filter { !order.contains($0.id) }
            }
            return ordered.filter { !removed.contains($0.id) }.map { step in
                var step = step
                if let text = edited[step.id] {
                    step.title = text.title
                    step.notes = text.notes
                }
                return step
            }
        }

        func edit(_ steps: [StepView]) -> PlanEdit {
            var edits = steps.compactMap { step in
                edited[step.id].map { PlanStepEdit(stepId: step.id, title: $0.title, notes: $0.notes, images: []) }
            }
            edits += added.map { PlanStepEdit(stepId: nil, title: $0.title, notes: $0.notes, images: []) }
            return PlanEdit(
                overview: nil,
                steps: edits,
                removeStepIds: steps.map(\.id).filter(removed.contains),
                order: (order ?? []).filter { !removed.contains($0) }
            )
        }
    }

    init(app: AppModel, taskId: String) {
        self.app = app
        self.taskId = taskId
        super.init(nibName: nil, bundle: nil)
        title = missionLabels().missionPage
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
        collectionView.allowsSelectionDuringEditing = true
        collectionView.accessibilityIdentifier = "mission-control"
        view.addSubview(collectionView)
        plan.onHeight = { [weak self] in
            guard let self, var snap = self.dataSource?.snapshot(), let row = snap.itemIdentifiers.first(where: { $0.id == "plan-text" }) else { return }
            snap.reconfigureItems([row])
            self.dataSource.apply(snap, animatingDifferences: false)
        }
        let planCell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { [weak self] cell, _, _ in
            guard let self else { return }
            cell.contentConfiguration = nil
            if self.plan.superview !== cell.contentView {
                self.plan.removeFromSuperview()
                self.plan.translatesAutoresizingMaskIntoConstraints = false
                cell.contentView.addSubview(self.plan)
                let margins = cell.contentView.layoutMarginsGuide
                NSLayoutConstraint.activate([
                    self.plan.topAnchor.constraint(equalTo: margins.topAnchor),
                    self.plan.bottomAnchor.constraint(equalTo: margins.bottomAnchor),
                    self.plan.leadingAnchor.constraint(equalTo: margins.leadingAnchor),
                    self.plan.trailingAnchor.constraint(equalTo: margins.trailingAnchor),
                ])
            }
            var bg = UIBackgroundConfiguration.listCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
        }
        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { [weak self] cell, _, row in
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
            if row.editable {
                accessories.append(.delete(displayed: .whenEditing) { [weak self] in self?.remove(row.id) })
                if row.id.hasPrefix("step:") { accessories.append(.reorder(displayed: .whenEditing)) }
            }
            if row.id.hasPrefix("step:"), self?.editingPlan != true { accessories.append(.disclosureIndicator()) }
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
            row.id == "plan-text"
                ? cv.dequeueConfiguredReusableCell(using: planCell, for: path, item: row)
                : cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        dataSource.supplementaryViewProvider = { cv, _, path in
            cv.dequeueConfiguredReusableSupplementary(using: header, for: path)
        }
        dataSource.reorderingHandlers.canReorderItem = { [weak self] row in
            self?.editingPlan == true && row.editable && row.id.hasPrefix("step:")
        }
        dataSource.reorderingHandlers.didReorder = { [weak self] transaction in
            guard let self, let detail = self.detail else { return }
            let ids = transaction.finalSnapshot.itemIdentifiers.compactMap { $0.id.hasPrefix("step:") ? String($0.id.dropFirst("step:".count)) : nil }
            let removed = detail.steps.map(\.id).filter(self.draft.removed.contains)
            let order = ids + removed
            self.draft.order = order == detail.steps.map(\.id) ? nil : order
            self.reload()
        }
        reload()
        appToken = app.observe { [weak self] in self?.reload() }
    }

    private var shownSteps: [StepView] { draft.shown(detail?.steps ?? []) }

    private func reload() {
        guard let detail = app.client?.taskDetail(taskId: taskId) else {
            navigationController?.popViewController(animated: true)
            return
        }
        self.detail = detail
        refreshEstimate(detail)
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

        let ready = detail.phase == .planReady
        if ready || !detail.plan.isEmpty {
            snap.appendSections([ready ? "Plan review" : "Plan"])
            var rows: [Row] = []
            if ready {
                rows.append(Row(
                    id: "plan",
                    title: "Plan ready · \(detail.steps.count) step\(detail.steps.count == 1 ? "" : "s") · the lead waits for you",
                    glyph: .symbol("doc.text")
                ))
            }
            rows += costRow
            if !detail.plan.isEmpty {
                plan.show(detail.plan)
                rows.append(Row(id: "plan-text", title: ""))
            }
            if ready {
                rows.append(Row(id: "approve", title: draft.count > 0 ? "Approve and run with \(changes)" : "Approve and run", glyph: .symbol("play.fill"), tint: Palette.accent, action: true))
            }
            snap.appendItems(rows)
        }

        let steps = shownSteps
        if !steps.isEmpty || editingPlan {
            snap.appendSections(["Steps · \(steps.count + draft.added.count)"])
            snap.appendItems(steps.map { step in
                var sub = [TaskGlyph.word(step.status)]
                if let question = step.question { sub.append(question) }
                if draft.edited[step.id] != nil { sub.append("Edited") }
                let line = estimate?.lines.first { $0.stepId == step.id }
                if let line { sub += [line.runsOn, line.multiplier].compactMap { $0 } }
                return Row(
                    id: "step:\(step.id)",
                    title: "\(step.label) · \(step.title)",
                    subtitle: ([sub.joined(separator: " · ")] + [line?.fallback].compactMap { $0 }).joined(separator: "\n"),
                    glyph: .step(step.status),
                    brand: step.brand,
                    editable: step.status == .planned
                )
            })
            snap.appendItems(draft.added.enumerated().map { ix, text in
                Row(id: "new:\(ix)", title: text.title, subtitle: "New step", glyph: .symbol("plus.circle"), editable: true)
            })
            if editingPlan {
                snap.appendItems([Row(id: "add-step", title: "Add step", glyph: .symbol("plus"), tint: Palette.accent, action: true)])
            }
        }

        if draft.count > 0 {
            snap.appendSections([" draft"])
            var rows = [Row(id: "send-draft", title: "Send \(changes) to the lead", glyph: .symbol("paperplane"), tint: Palette.accent, action: true)]
            rows.append(Row(id: "discard-draft", title: "Discard changes", glyph: .symbol("xmark"), tint: Palette.danger, action: true))
            snap.appendItems(rows)
        }

        if let lead = task.leadChatId {
            snap.appendSections([" lead"])
            snap.appendItems([Row(id: "lead:\(lead)", title: "Open lead thread", glyph: .symbol("bubble.left"), tint: Palette.accent, action: true)])
        }
        snap.reconfigureItems(snap.itemIdentifiers.filter { $0.id.hasPrefix("step:") || $0.id.hasPrefix("new:") })
        dataSource.apply(snap, animatingDifferences: view.window != nil)
        updateBar(detail)
        if let lead = task.leadChatId, navigationItem.titleView == nil {
            let views = UISegmentedControl(items: Array(treeLabels().views.prefix(2)))
            views.selectedSegmentIndex = 0
            views.accessibilityIdentifier = "mc-tree-view"
            views.addAction(UIAction { [weak self, weak views] _ in
                self?.showOutline(views?.selectedSegmentIndex == 1 ? lead : nil)
            }, for: .valueChanged)
            navigationItem.titleView = views
        }

        let held = detail.approval?.held == true
        if held, countdown == nil {
            countdown = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.reload() }
        } else if !held {
            countdown?.invalidate()
            countdown = nil
        }
    }

    private func showOutline(_ lead: String?) {
        outline?.willMove(toParent: nil)
        outline?.view.removeFromSuperview()
        outline?.removeFromParent()
        outline = nil
        collectionView.isHidden = lead != nil
        guard let lead else { return }
        let tree = TreeOutlineViewController(app: app, rootChatId: lead, title: detail?.row.title)
        addChild(tree)
        tree.view.frame = view.bounds
        tree.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(tree.view)
        tree.didMove(toParent: self)
        outline = tree
    }

    private func refreshEstimate(_ detail: TaskDetail) {
        let key = detail.steps.map { "\($0.id)=\($0.status)" }.joined(separator: ",")
        if key != estimateKey, let client = app.client {
            estimateKey = key
            let id = taskId
            Task { @MainActor [weak self] in
                let estimate = try? await client.planEstimate(taskId: id)
                guard let self, self.estimateKey == key else { return }
                self.estimate = estimate ?? nil
                self.reload()
            }
        }
    }

    private var costRow: [Row] {
        guard let estimate, !estimate.lines.isEmpty else { return [] }
        return [Row(
            id: "est-cost",
            title: "\(estimate.cost) \(estimate.baseline)",
            subtitle: ([estimate.mix.joined(separator: ", ")] + [estimate.raised].compactMap { $0 }).joined(separator: "\n"),
            glyph: .symbol("gauge.with.dots.needle.33percent"),
            tint: estimate.raised == nil ? Palette.text : Palette.warning
        )]
    }

    private var changes: String { draft.count == 1 ? "1 change" : "\(draft.count) changes" }

    private func updateBar(_ detail: TaskDetail) {
        var items: [UIBarButtonItem] = []
        let menu = UIBarButtonItem(image: UIImage(systemName: "ellipsis"), menu: missionMenu(detail))
        menu.accessibilityIdentifier = "mc-menu"
        items.append(menu)
        if detail.steps.contains(where: { $0.status == .planned }) || editingPlan {
            let edit = UIBarButtonItem(title: editingPlan ? "Done" : "Edit", primaryAction: UIAction { [weak self] _ in self?.toggleEditing() })
            edit.accessibilityIdentifier = "mc-edit"
            items.append(edit)
        }
        if navigationItem.rightBarButtonItems?.map(\.title) != items.map(\.title) {
            navigationItem.setRightBarButtonItems(items, animated: false)
        } else {
            navigationItem.rightBarButtonItems?.first?.menu = menu.menu
        }
    }

    private func missionMenu(_ detail: TaskDetail) -> UIMenu {
        var children: [UIMenuElement] = []
        if let watch = detail.watch, let lead = detail.row.leadChatId, let client = app.client {
            children.append(UIAction(title: "Watched by the Supervisor", image: UIImage(systemName: "eye"), state: watch.watched ? .on : .off) { [weak self] _ in
                self?.run { try await client.setWatched(chatId: lead, watched: !watch.watched) }
            })
            children.append(UIAction(title: "Authority on \(watch.host.name)", image: UIImage(systemName: "slider.horizontal.3")) { [weak self] _ in
                self?.presentAuthority(watch.host)
            })
        }
        children.append(UIAction(title: "Open Supervisor", image: TaskGlyph.asset("glyph-supervisor")) { [weak self] _ in
            guard let self else { return }
            self.navigationController?.pushViewController(SupervisorViewController(app: self.app), animated: true)
        })
        if detail.row.leadChatId != nil {
            children.append(UIAction(title: "Lead brief", image: UIImage(systemName: "doc.plaintext")) { [weak self] _ in
                self?.openBrief(stepId: nil)
            })
        }
        return UIMenu(children: children)
    }

    private func presentAuthority(_ host: HostRef) {
        let nav = UINavigationController(rootViewController: AuthorityViewController(app: app, host: host))
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.large()]
            sheet.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }

    private func openBrief(stepId: String?) {
        navigationController?.pushViewController(BriefViewController(app: app, taskId: taskId, stepId: stepId), animated: true)
    }

    private func toggleEditing() {
        editingPlan.toggle()
        collectionView.isEditing = editingPlan
        reload()
    }

    private func remove(_ rowId: String) {
        if rowId.hasPrefix("step:") {
            draft.removed.insert(String(rowId.dropFirst("step:".count)))
        } else if rowId.hasPrefix("new:"), let ix = Int(rowId.dropFirst("new:".count)), draft.added.indices.contains(ix) {
            draft.added.remove(at: ix)
        }
        reload()
    }

    private func editText(title: String, current: StepText?, save: @escaping (StepText) -> Void) {
        let alert = UIAlertController(title: title, message: nil, preferredStyle: .alert)
        alert.addTextField { field in
            field.placeholder = "Title"
            field.text = current?.title
            field.accessibilityIdentifier = "step-title"
        }
        alert.addTextField { field in
            field.placeholder = "Notes"
            field.text = current?.notes
            field.accessibilityIdentifier = "step-notes"
        }
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: "Save", style: .default) { [weak alert] _ in
            let fields = alert?.textFields ?? []
            let text = StepText(title: fields.first?.text ?? "", notes: fields.last?.text ?? "")
            guard !text.title.trimmingCharacters(in: .whitespaces).isEmpty else { return }
            save(text)
        })
        present(alert, animated: true)
    }

    private func sendDraft(approve: Bool) {
        guard let client = app.client, let detail else { return }
        let edit = draft.count > 0 ? draft.edit(detail.steps) : nil
        let id = taskId
        Task { @MainActor in
            do {
                try await client.editTaskPlan(taskId: id, edit: edit, approve: approve)
                draft = Draft()
                if editingPlan { toggleEditing() } else { reload() }
            } catch {
                Toast.show(TasksErrors.message(error), in: view.window)
            }
        }
    }

    func collectionView(_ collectionView: UICollectionView, targetIndexPathForMoveOfItemFromOriginalIndexPath originalIndexPath: IndexPath, atCurrentIndexPath currentIndexPath: IndexPath, toProposedIndexPath proposedIndexPath: IndexPath) -> IndexPath {
        let section = originalIndexPath.section
        let rows = dataSource.snapshot().itemIdentifiers(inSection: dataSource.snapshot().sectionIdentifiers[section])
        let movable = rows.indices.filter { rows[$0].id.hasPrefix("step:") && rows[$0].editable }
        guard let lo = movable.first, let hi = movable.last else { return originalIndexPath }
        let target = proposedIndexPath.section == section ? proposedIndexPath.item : (proposedIndexPath.section < section ? lo : hi)
        return IndexPath(item: min(max(target, lo), hi), section: section)
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        guard let row = dataSource.itemIdentifier(for: indexPath) else { return false }
        if editingPlan { return row.action || row.editable }
        return row.action || row.id.hasPrefix("step:")
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let row = dataSource.itemIdentifier(for: indexPath), let client = app.client else { return }
        let id = taskId
        switch row.id {
        case "approve":
            sendDraft(approve: true)
        case "send-draft":
            sendDraft(approve: false)
        case "discard-draft":
            draft = Draft()
            reload()
        case "add-step":
            editText(title: "New step", current: nil) { [weak self] text in
                self?.draft.added.append(text)
                self?.reload()
            }
        case "take-back":
            run { try await client.takeBackPlanApproval(taskId: id) }
        case let lead where lead.hasPrefix("lead:"):
            TasksViewController.push(thread: String(lead.dropFirst("lead:".count)), app: app, from: navigationController)
        case let added where added.hasPrefix("new:"):
            guard let ix = Int(added.dropFirst("new:".count)), draft.added.indices.contains(ix) else { return }
            editText(title: "Edit step", current: draft.added[ix]) { [weak self] text in
                self?.draft.added[ix] = text
                self?.reload()
            }
        case let step where step.hasPrefix("step:"):
            guard let view = shownSteps.first(where: { "step:\($0.id)" == step }) else { return }
            if editingPlan {
                editText(title: "Edit \(view.label)", current: StepText(title: view.title, notes: view.notes)) { [weak self] text in
                    self?.draft.edited[view.id] = text
                    self?.reload()
                }
            } else {
                stepActions(view, client: client, from: collectionView.cellForItem(at: indexPath))
            }
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
        sheet.addAction(UIAlertAction(title: "Brief", style: .default) { [weak self] _ in
            self?.openBrief(stepId: step.id)
        })
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
