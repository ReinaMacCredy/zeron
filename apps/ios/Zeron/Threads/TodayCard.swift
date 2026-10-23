import UIKit

enum TodayText {
    static func headline(_ page: TodayPage, size: CGFloat) -> NSAttributedString {
        let out = NSMutableAttributedString(string: page.greeting + ". ", attributes: [.font: Fonts.ui(.sansSemibold, size), .foregroundColor: Palette.text])
        out.append(NSAttributedString(string: page.summary, attributes: [.font: Fonts.ui(.sans, size), .foregroundColor: Palette.secondary]))
        return out
    }

    static func digest(_ page: TodayPage) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for (i, sentence) in page.digest.enumerated() {
            if i > 0 { out.append(NSAttributedString(string: " ")) }
            for part in sentence.parts {
                switch part {
                case let .text(text):
                    out.append(NSAttributedString(string: text, attributes: [.font: Fonts.ui(.sans, 14), .foregroundColor: Palette.text]))
                case let .strong(text):
                    out.append(NSAttributedString(string: text, attributes: [.font: Fonts.ui(.sansSemibold, 14), .foregroundColor: Palette.text]))
                case let .bad(text):
                    out.append(NSAttributedString(string: text, attributes: [.font: Fonts.ui(.sansSemibold, 14), .foregroundColor: Palette.danger]))
                }
            }
        }
        return out
    }

    static func usageLead(_ usage: TodayUsage) -> String {
        "Left on \(usage.harnessName), \(usage.window.lowercased())"
    }

    static func remaining(_ usage: TodayUsage) -> Int {
        Int(((1 - usage.usedFraction) * 100).rounded())
    }

    static func usageTone(_ usage: TodayUsage) -> UIColor {
        usage.usedFraction >= 0.95 ? Palette.danger : usage.usedFraction >= 0.8 ? Palette.warning : Palette.accent
    }
}

final class TodayCardCell: UICollectionViewListCell {
    private let card = UIView()
    private let headline = UILabel()
    private let tiles = UIStackView()

    override init(frame: CGRect) {
        super.init(frame: frame)
        var bg = UIBackgroundConfiguration.clear()
        bg.backgroundColor = .clear
        backgroundConfiguration = bg
        accessibilityIdentifier = "today-card"
        card.backgroundColor = Palette.elevated
        card.layer.cornerRadius = 16
        card.layer.cornerCurve = .continuous
        headline.numberOfLines = 0
        tiles.axis = .horizontal
        tiles.spacing = 6
        tiles.distribution = .fillEqually
        let stack = UIStackView(arrangedSubviews: [headline, tiles])
        stack.axis = .vertical
        stack.spacing = 10
        for v in [card, stack] { v.translatesAutoresizingMaskIntoConstraints = false }
        contentView.addSubview(card)
        card.addSubview(stack)
        NSLayoutConstraint.activate([
            card.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 6),
            card.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -6),
            card.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            card.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            stack.topAnchor.constraint(equalTo: card.topAnchor, constant: 14),
            stack.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -12),
            stack.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 14),
            stack.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -14),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(_ page: TodayPage) {
        headline.attributedText = TodayText.headline(page, size: 15)
        tiles.arrangedSubviews.forEach { $0.removeFromSuperview() }
        tiles.addArrangedSubview(Self.tile("Today's tasks", "\(page.openTasks)", "open"))
        tiles.addArrangedSubview(Self.tile("Your day so far", "\(page.prompts)", page.prompts == 1 ? "prompt" : "prompts"))
        if let usage = page.usage {
            tiles.addArrangedSubview(Self.tile(TodayText.usageLead(usage), "\(TodayText.remaining(usage))%", nil))
        }
        accessibilityLabel = "Today. \(page.greeting). \(page.summary)"
    }

    private static func tile(_ key: String, _ value: String, _ unit: String?) -> UIView {
        let box = UIView()
        box.backgroundColor = Palette.background
        box.layer.cornerRadius = 10
        box.layer.cornerCurve = .continuous
        let k = UILabel()
        k.text = key
        k.font = Fonts.ui(.sans, 11)
        k.textColor = Palette.secondary
        k.numberOfLines = 2
        let v = UILabel()
        let text = NSMutableAttributedString(string: value, attributes: [.font: Fonts.ui(.sansBold, 17), .foregroundColor: Palette.text])
        if let unit {
            text.append(NSAttributedString(string: " " + unit, attributes: [.font: Fonts.ui(.sansMedium, 11), .foregroundColor: Palette.secondary]))
        }
        v.attributedText = text
        let stack = UIStackView(arrangedSubviews: [k, UIView(), v])
        stack.axis = .vertical
        stack.spacing = 2
        stack.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(stack)
        NSLayoutConstraint.activate([
            box.heightAnchor.constraint(greaterThanOrEqualToConstant: 62),
            stack.topAnchor.constraint(equalTo: box.topAnchor, constant: 7),
            stack.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -7),
            stack.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 8),
            stack.trailingAnchor.constraint(equalTo: box.trailingAnchor, constant: -8),
        ])
        return box
    }
}

final class TodaySheetController: UITableViewController, UITextFieldDelegate {
    private enum Section { case tasks, day, usage }

    private let app: AppModel
    private weak var host: UIViewController?
    private var page: TodayPage
    private var sections: [Section] = []
    private var token: AnyObject?
    private var pending: [String: Bool] = [:]

    static func present(app: AppModel, from host: UIViewController) {
        let sheet = TodaySheetController(app: app)
        sheet.host = host
        let nav = UINavigationController(rootViewController: sheet)
        nav.navigationBar.titleTextAttributes = [.font: Fonts.ui(.sansSemibold, 16), .foregroundColor: Palette.text]
        nav.navigationBar.tintColor = Palette.text
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.large()]
            sheet.prefersGrabberVisible = true
        }
        host.present(nav, animated: true)
        app.refreshToday()
    }

    init(app: AppModel) {
        self.app = app
        page = app.today
        super.init(style: .insetGrouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Today"
        tableView.backgroundColor = Palette.background
        tableView.accessibilityIdentifier = "today-sheet"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        token = app.observe { [weak self] in self?.reload() }
        reload()
    }

    private var reloadAfterEditing = false

    private func reload() {
        // Rebuilding the rows would take the add field's keyboard away mid-word.
        if addField.isFirstResponder {
            reloadAfterEditing = true
            return
        }
        reloadAfterEditing = false
        page = app.today
        pending = pending.filter { id, done in page.tasks.first { $0.id == id }?.done != done }
        sections = [.tasks, .day] + (page.usage == nil ? [] : [.usage])
        tableView.reloadData()
    }

    override func numberOfSections(in tableView: UITableView) -> Int { sections.count }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        sections[section] == .tasks ? max(1, page.tasks.count) + 1 : 1
    }

    override func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
        let title = UILabel()
        title.font = Fonts.ui(.sansSemibold, 13)
        title.textColor = Palette.secondary
        let row = UIStackView(arrangedSubviews: [title])
        row.alignment = .firstBaseline
        switch sections[section] {
        case .tasks:
            title.text = "Today's tasks · \(page.openTasks)"
            let all = UIButton(type: .system, primaryAction: UIAction(title: "All tasks") { [weak self] _ in self?.openAllTasks() })
            all.titleLabel?.font = Fonts.ui(.sansMedium, 13)
            all.tintColor = Palette.accent
            all.accessibilityIdentifier = "today-all-tasks"
            row.addArrangedSubview(all)
        case .day:
            title.text = "Your day so far"
            if let updated = page.updated {
                let stamp = UILabel()
                stamp.text = updated
                stamp.font = Fonts.ui(.sansMedium, 12)
                stamp.textColor = Palette.tertiary
                stamp.accessibilityIdentifier = "today-updated"
                row.addArrangedSubview(stamp)
            }
        case .usage:
            title.text = "Left on usage"
        }
        let header = UIView()
        row.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(row)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: header.topAnchor, constant: 14),
            row.bottomAnchor.constraint(equalTo: header.bottomAnchor, constant: -6),
            row.leadingAnchor.constraint(equalTo: header.layoutMarginsGuide.leadingAnchor),
            row.trailingAnchor.constraint(equalTo: header.layoutMarginsGuide.trailingAnchor),
        ])
        return header
    }

    private func openAllTasks() {
        let tabs = host?.tabBarController as? MainTabController
        dismiss(animated: true) { tabs?.showTasks("") }
    }

    private lazy var addField: UITextField = {
        let field = UITextField()
        field.placeholder = "Add a task for today"
        field.font = Fonts.ui(.sans, 15)
        field.textColor = Palette.text
        field.returnKeyType = .done
        field.delegate = self
        field.accessibilityIdentifier = "today-add-task"
        return field
    }()

    func textFieldDidEndEditing(_ textField: UITextField) {
        if reloadAfterEditing { reload() }
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        let title = textField.text?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !title.isEmpty else {
            textField.resignFirstResponder()
            return false
        }
        textField.text = ""
        textField.resignFirstResponder()
        Task { @MainActor [weak self] in
            do {
                try await self?.app.addTodayTask(title)
            } catch {
                if self?.addField.text?.isEmpty == true { self?.addField.text = title }
            }
        }
        return false
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        cell.accessoryView = nil
        cell.selectionStyle = .none
        cell.accessibilityIdentifier = nil
        cell.accessibilityValue = nil
        var c = UIListContentConfiguration.subtitleCell()
        c.textProperties.font = Fonts.ui(.sans, 15)
        c.textProperties.color = Palette.text
        c.secondaryTextProperties.font = Fonts.ui(.sans, 12)
        c.secondaryTextProperties.color = Palette.secondary
        switch sections[indexPath.section] {
        case .tasks where indexPath.row == tableView.numberOfRows(inSection: indexPath.section) - 1:
            cell.contentConfiguration = nil
            cell.contentView.subviews.forEach { $0.removeFromSuperview() }
            let mark = UIImageView(image: UIImage(systemName: "circle.dashed"))
            mark.tintColor = Palette.tertiary
            mark.setContentHuggingPriority(.required, for: .horizontal)
            let row = UIStackView(arrangedSubviews: [mark, addField])
            row.spacing = 12
            row.alignment = .center
            embed(row, in: cell)
            return cell
        case .tasks:
            guard indexPath.row < page.tasks.count else {
                c.text = "Nothing due today"
                c.textProperties.color = Palette.secondary
                break
            }
            let task = page.tasks[indexPath.row]
            let done = pending[task.id] ?? task.done
            c.image = UIImage(systemName: done ? "checkmark.circle.fill" : "circle")
            c.imageProperties.tintColor = done ? StatusTone.done : Palette.tertiary
            c.attributedText = NSAttributedString(string: task.title, attributes: [
                .font: Fonts.ui(.sans, 15),
                .foregroundColor: done ? Palette.tertiary : Palette.text,
                .strikethroughStyle: done ? NSUnderlineStyle.single.rawValue : 0,
            ])
            let host = task.host.map { " · \($0.name)" } ?? ""
            c.secondaryText = task.meta.map { $0 + host } ?? (host.isEmpty ? nil : String(host.dropFirst(3)))
            c.secondaryTextProperties.color = task.overdue && !done ? Palette.danger : Palette.secondary
            cell.selectionStyle = .default
            cell.accessibilityIdentifier = "today-task-\(task.id)"
            cell.accessibilityValue = done ? "Done" : "Open"
        case .day:
            cell.contentConfiguration = nil
            cell.contentView.subviews.forEach { $0.removeFromSuperview() }
            embed(dayView(), in: cell)
            return cell
        case .usage:
            cell.contentConfiguration = nil
            cell.contentView.subviews.forEach { $0.removeFromSuperview() }
            if let usage = page.usage { embed(usageView(usage), in: cell) }
            return cell
        }
        cell.contentView.subviews.filter { $0.tag == 7 }.forEach { $0.removeFromSuperview() }
        cell.contentConfiguration = c
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        guard sections[indexPath.section] == .tasks, indexPath.row < page.tasks.count, let client = app.client else { return }
        let task = page.tasks[indexPath.row]
        let done = !(pending[task.id] ?? task.done)
        pending[task.id] = done
        UISelectionFeedbackGenerator().selectionChanged()
        tableView.reloadRows(at: [indexPath], with: .none)
        Task { @MainActor [weak self] in
            do {
                try await client.setTaskCompleted(taskId: task.id, completed: done)
            } catch {
                self?.pending[task.id] = nil
                self?.reload()
            }
        }
    }

    private func embed(_ view: UIView, in cell: UITableViewCell) {
        view.tag = 7
        view.translatesAutoresizingMaskIntoConstraints = false
        cell.contentView.addSubview(view)
        NSLayoutConstraint.activate([
            view.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 12),
            view.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -12),
            view.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
            view.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
        ])
    }

    private func dayView() -> UIView {
        let text = UILabel()
        text.numberOfLines = 0
        text.attributedText = page.runsLoaded || !page.digest.isEmpty
            ? TodayText.digest(page)
            : NSAttributedString(string: "Loading your day…", attributes: [.font: Fonts.ui(.sans, 14), .foregroundColor: Palette.secondary])
        let peak = max(1, page.slots.map(\.prompts).max() ?? 0)
        let slots = UIStackView(arrangedSubviews: page.slots.map { slot in
            let label = UILabel()
            label.text = slot.now ? "\(slot.label), now" : slot.label
            label.font = Fonts.ui(slot.now ? .sansSemibold : .sans, 11)
            label.textColor = Palette.secondary
            let count = UILabel()
            count.text = slot.prompts == 0 ? " " : "\(slot.prompts) \(slot.prompts == 1 ? "prompt" : "prompts")"
            count.font = Fonts.ui(.sans, 11)
            count.textColor = Palette.secondary
            let column = UIStackView(arrangedSubviews: [label, Self.bar(CGFloat(slot.prompts) / CGFloat(peak), tone: Palette.accent), count])
            column.axis = .vertical
            column.spacing = 4
            column.accessibilityIdentifier = "today-slot-\(slot.label)"
            return column
        })
        slots.axis = .horizontal
        slots.spacing = 10
        slots.distribution = .fillEqually
        let stack = UIStackView(arrangedSubviews: [text, slots])
        stack.axis = .vertical
        stack.spacing = 12
        return stack
    }

    private func usageView(_ usage: TodayUsage) -> UIView {
        let mark = UIImageView(image: BrandMarks.image(for: usage.harness, side: 18))
        mark.contentMode = .scaleAspectFit
        NSLayoutConstraint.activate([mark.widthAnchor.constraint(equalToConstant: 18), mark.heightAnchor.constraint(equalToConstant: 18)])
        let lead = UILabel()
        lead.text = TodayText.usageLead(usage)
        lead.font = Fonts.ui(.sans, 14)
        lead.textColor = Palette.text
        let value = UILabel()
        value.text = "\(TodayText.remaining(usage))%"
        value.font = Fonts.ui(.sansBold, 17)
        value.textColor = Palette.text
        value.setContentHuggingPriority(.required, for: .horizontal)
        let head = UIStackView(arrangedSubviews: [mark, lead, value])
        head.spacing = 8
        head.alignment = .center
        let detail = UILabel()
        detail.text = [usage.resetsIn.map { "Resets in \($0)" } ?? "Your tightest limit", usage.host.name].joined(separator: " · ")
        detail.font = Fonts.ui(.sans, 12)
        detail.textColor = Palette.secondary
        let stack = UIStackView(arrangedSubviews: [head, Self.bar(CGFloat(1 - usage.usedFraction), tone: TodayText.usageTone(usage)), detail])
        stack.axis = .vertical
        stack.spacing = 8
        stack.accessibilityIdentifier = "today-usage"
        return stack
    }

    private static func bar(_ fraction: CGFloat, tone: UIColor) -> UIView {
        let track = UIView()
        track.backgroundColor = Palette.controlFill
        track.layer.cornerRadius = 2.5
        let fill = UIView()
        fill.backgroundColor = tone
        fill.layer.cornerRadius = 2.5
        fill.translatesAutoresizingMaskIntoConstraints = false
        track.addSubview(fill)
        NSLayoutConstraint.activate([
            track.heightAnchor.constraint(equalToConstant: 5),
            fill.leadingAnchor.constraint(equalTo: track.leadingAnchor),
            fill.topAnchor.constraint(equalTo: track.topAnchor),
            fill.bottomAnchor.constraint(equalTo: track.bottomAnchor),
            fill.widthAnchor.constraint(equalTo: track.widthAnchor, multiplier: max(0.001, min(1, fraction))),
        ])
        return track
    }
}
