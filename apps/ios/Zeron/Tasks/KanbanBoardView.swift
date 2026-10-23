import UIKit

final class KanbanBoardView: UIScrollView {
    var onOpen: ((TaskRow) -> Void)?
    var onMove: ((TaskRow, Bool) -> Void)?
    private let columns = UIStackView()
    private var shown: [KanbanColumn] = []

    override init(frame: CGRect) {
        super.init(frame: frame)
        accessibilityIdentifier = "tasks-kanban"
        showsHorizontalScrollIndicator = false
        alwaysBounceHorizontal = true
        decelerationRate = .fast
        columns.axis = .horizontal
        columns.spacing = 12
        columns.alignment = .top
        columns.translatesAutoresizingMaskIntoConstraints = false
        addSubview(columns)
        NSLayoutConstraint.activate([
            columns.topAnchor.constraint(equalTo: contentLayoutGuide.topAnchor, constant: 8),
            columns.leadingAnchor.constraint(equalTo: contentLayoutGuide.leadingAnchor, constant: 16),
            columns.trailingAnchor.constraint(equalTo: contentLayoutGuide.trailingAnchor, constant: -16),
            columns.bottomAnchor.constraint(equalTo: contentLayoutGuide.bottomAnchor, constant: -16),
            columns.heightAnchor.constraint(greaterThanOrEqualTo: frameLayoutGuide.heightAnchor, constant: -24),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(_ board: [KanbanColumn]) {
        guard board != shown else { return }
        shown = board
        columns.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for column in board {
            let view = columnView(column)
            columns.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: frameLayoutGuide.widthAnchor, multiplier: 0.78).isActive = true
        }
    }

    private func columnView(_ column: KanbanColumn) -> UIView {
        let dot = UIView()
        dot.backgroundColor = Self.tone(column.status)
        dot.layer.cornerRadius = 4
        dot.widthAnchor.constraint(equalToConstant: 8).isActive = true
        dot.heightAnchor.constraint(equalToConstant: 8).isActive = true
        let title = UILabel()
        title.text = column.title
        title.font = Fonts.ui(.sansMedium, TypeScale.size(14))
        title.textColor = Palette.text
        let count = UILabel()
        count.text = "\(column.rows.count)"
        count.font = Fonts.ui(.sans, TypeScale.size(13))
        count.textColor = Palette.secondary
        let head = UIStackView(arrangedSubviews: [dot, title, count, UIView()])
        head.spacing = 8
        head.alignment = .center
        head.isLayoutMarginsRelativeArrangement = true
        head.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 4, leading: 6, bottom: 4, trailing: 6)
        let stack = UIStackView(arrangedSubviews: [head] + column.rows.map { card($0) })
        stack.axis = .vertical
        stack.spacing = 8
        stack.accessibilityIdentifier = "kanban-column-\(column.title)"
        return stack
    }

    private func card(_ row: TaskRow) -> UIView {
        let card = KanbanCard(row: row)
        card.addAction(UIAction { [weak self] _ in self?.onOpen?(row) }, for: .primaryActionTriggered)
        card.menu = { [weak self] in
            guard let self else { return nil }
            let moves = self.shown.compactMap { target -> UIAction? in
                guard let completes = taskDropCompletes(from: row.status, to: target.status) else { return nil }
                return UIAction(title: "Move to \(target.title)", image: UIImage(systemName: completes ? "checkmark.circle" : "arrow.uturn.backward.circle")) { [weak self] _ in
                    self?.onMove?(row, completes)
                }
            }
            let open = UIAction(title: "Open", image: UIImage(systemName: "arrow.up.right")) { [weak self] _ in self?.onOpen?(row) }
            return UIMenu(children: [open] + moves)
        }
        return card
    }

    static func tone(_ status: TaskStatus) -> UIColor {
        switch status {
        case .toDo: StatusTone.idle
        case .working: StatusTone.working
        case .needsYou: StatusTone.input
        case .done: StatusTone.done
        }
    }

    static func tone(_ status: StepStatus) -> UIColor {
        switch status {
        case .done: StatusTone.done
        case .working: StatusTone.working
        case .needsYou: Palette.warning
        case .failed: StatusTone.failed
        case .planned, .skipped: Palette.text.withAlphaComponent(0.10)
        }
    }
}

private final class KanbanCard: UIControl {
    var menu: (() -> UIMenu?)?

    init(row: TaskRow) {
        super.init(frame: .zero)
        backgroundColor = Palette.elevated
        layer.cornerRadius = 10
        layer.cornerCurve = .continuous
        layer.borderWidth = 1
        layer.borderColor = Palette.hairline.cgColor
        accessibilityIdentifier = "kanban-card-\(row.id)"
        isAccessibilityElement = true
        accessibilityLabel = row.title
        accessibilityTraits = .button
        isContextMenuInteractionEnabled = true

        let title = UILabel()
        title.text = row.title
        title.numberOfLines = 3
        title.font = Fonts.ui(.sansMedium, TypeScale.size(14))
        title.textColor = row.status == .done ? Palette.secondary : Palette.text
        var views: [UIView] = [title]
        if row.mission, !row.stepBar.isEmpty {
            let bar = UIStackView(arrangedSubviews: row.stepBar.map { status in
                let segment = UIView()
                segment.backgroundColor = KanbanBoardView.tone(status)
                segment.layer.cornerRadius = 2
                segment.heightAnchor.constraint(equalToConstant: 4).isActive = true
                return segment
            })
            bar.spacing = 2
            bar.distribution = .fillEqually
            bar.accessibilityIdentifier = "task-step-bar-\(row.id)"
            views.append(bar)
        }
        let footer = UILabel()
        footer.text = row.mission ? TasksViewController.missionLine(row) : TasksViewController.taskLine(row)
        footer.font = Fonts.ui(.sans, TypeScale.size(12))
        footer.textColor = Palette.secondary
        footer.numberOfLines = 1
        views.append(footer)
        let stack = UIStackView(arrangedSubviews: views)
        stack.axis = .vertical
        stack.spacing = 8
        stack.isUserInteractionEnabled = false
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isHighlighted: Bool {
        didSet { alpha = isHighlighted ? 0.7 : 1 }
    }

    override func contextMenuInteraction(_ interaction: UIContextMenuInteraction, configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
        UIContextMenuConfiguration(actionProvider: { [weak self] _ in self?.menu?() })
    }
}
