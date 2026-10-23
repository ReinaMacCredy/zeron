import UIKit

final class ContextSheetViewController: UITableViewController {
    struct Content: Equatable {
        var card: ContextCard?
        var stats: SessionStatsView?
        var compact: CompactSectionView?
    }

    private let content: () -> Content
    private let toggleLong: () -> Void
    private let pick: (UInt32) -> Void
    private var shown = Content()
    private var ticker: Timer?

    init(content: @escaping () -> Content, toggleLong: @escaping () -> Void, pick: @escaping (UInt32) -> Void) {
        self.content = content
        self.toggleLong = toggleLong
        self.pick = pick
        super.init(style: .insetGrouped)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var wrapped: UINavigationController {
        let nav = UINavigationController(rootViewController: self)
        nav.modalPresentationStyle = .pageSheet
        if let s = nav.sheetPresentationController {
            s.detents = [.medium(), .large()]
            s.prefersGrabberVisible = true
        }
        return nav
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Context and stats"
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "cell")
        shown = content()
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        ticker = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.reload() }
        }
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        ticker?.invalidate()
        ticker = nil
    }

    func reload() {
        let next = content()
        guard next != shown else { return }
        shown = next
        tableView.reloadData()
    }

    private enum Section { case context, compact, cost, stats }
    private enum Row { case long, track, readout, cost, stat(StatsRow) }

    private var sections: [Section] {
        var out: [Section] = []
        if shown.card != nil { out.append(.context) }
        if shown.compact != nil { out.append(.compact) }
        if shown.stats?.cost != nil { out.append(.cost) }
        if shown.stats != nil { out.append(.stats) }
        return out
    }

    private func rows(_ section: Section) -> [Row] {
        switch section {
        case .context: return [.long, .track, .readout]
        case .compact: return (shown.compact?.rows ?? []).map(Row.stat)
        case .cost: return [.cost] + (shown.stats?.split ?? []).map(Row.stat)
        case .stats: return (shown.stats?.rows ?? []).map(Row.stat)
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { sections.count }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        rows(sections[section]).count
    }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        switch sections[section] {
        case .context: return "Context"
        case .compact: return shown.compact?.heading
        case .cost: return "Session cost"
        case .stats: return "Session statistics"
        }
    }

    override func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        switch sections[section] {
        case .context: return shown.card?.warning
        case .compact: return shown.compact?.note
        case .cost: return shown.stats?.costNote
        case .stats: return nil
        }
    }

    override func tableView(_ tableView: UITableView, willDisplayFooterView view: UIView, forSection section: Int) {
        guard sections[section] == .context else { return }
        (view as? UITableViewHeaderFooterView)?.textLabel?.textColor = Palette.warning
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "cell", for: indexPath)
        cell.selectionStyle = .none
        cell.accessoryView = nil
        cell.contentConfiguration = nil
        cell.contentView.subviews.filter { $0.tag == 7 }.forEach { $0.removeFromSuperview() }
        switch rows(sections[indexPath.section])[indexPath.row] {
        case .cost:
            guard let stats = shown.stats else { break }
            var c = UIListContentConfiguration.valueCell()
            c.text = stats.cost
            c.textProperties.font = .preferredFont(forTextStyle: stats.priced ? .title2 : .body)
            c.textProperties.color = stats.priced ? .label : Palette.secondary
            c.secondaryText = stats.tokens
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "stats-cost"
        case .stat(let row):
            var c = UIListContentConfiguration.valueCell()
            c.text = row.label
            c.secondaryText = row.value
            cell.contentConfiguration = c
        case .long:
            guard let card = shown.card else { break }
            var c = cell.defaultContentConfiguration()
            c.text = "Long context"
            cell.contentConfiguration = c
            let toggle = UISwitch()
            toggle.isOn = card.longContext
            toggle.accessibilityIdentifier = "context-long"
            toggle.addAction(UIAction { [weak self] _ in self?.toggleLong() }, for: .valueChanged)
            cell.accessoryView = toggle
        case .track:
            guard let card = shown.card else { break }
            let track = UISegmentedControl(items: [card.shortLabel] + card.stops)
            track.tag = 7
            track.selectedSegmentIndex = Int(card.position)
            track.accessibilityIdentifier = "context-track"
            track.addAction(UIAction { [weak self, weak track] _ in
                guard let track else { return }
                self?.pick(UInt32(track.selectedSegmentIndex))
            }, for: .valueChanged)
            track.translatesAutoresizingMaskIntoConstraints = false
            cell.contentView.addSubview(track)
            NSLayoutConstraint.activate([
                track.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
                track.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
                track.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 10),
                track.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -10),
            ])
        case .readout:
            guard let card = shown.card else { break }
            var c = cell.defaultContentConfiguration()
            let text = NSMutableAttributedString(string: card.windowLabel, attributes: [.foregroundColor: card.longContext ? Palette.secondary : Palette.warning])
            text.append(NSAttributedString(string: " · \(card.thresholdLabel)", attributes: [.foregroundColor: Palette.secondary]))
            c.attributedText = text
            c.textProperties.font = .preferredFont(forTextStyle: .footnote)
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "context-readout"
        }
        return cell
    }
}
