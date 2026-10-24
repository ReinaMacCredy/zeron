import UIKit

final class ClaudeConfigSheet: UITableViewController, UISearchTextFieldDelegate, UIAdaptivePresentationControllerDelegate {
    var applyChip: ((ChipWrite) async -> Void)?
    var openUsage: (() -> Void)?
    var onCloseFailed: ((String) -> Void)?

    private let panel: ClaudeConfigPanel
    private var current: ConfigTab = .config
    private var filter: ConfigFilter = .all
    private var query = ""
    private var list = ConfigList(message: nil, rows: [], shown: "", footer: "")
    private var expanded: String?
    private var selected: String?
    private var statsRequested = false
    private var finished = false
    private let tabs = UISegmentedControl()
    private let search = UISearchTextField()
    private let chips = UIStackView()
    private let shown = UILabel()
    private let footer = UILabel()

    init(panel: ClaudeConfigPanel, tab: ConfigTab = .config, query: String = "") {
        self.panel = panel
        self.current = tab
        self.query = query
        super.init(style: .insetGrouped)
        title = "Claude Code settings"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = Palette.background
        tableView.keyboardDismissMode = .onDrag
        tableView.accessibilityIdentifier = "claude-config"
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true) { self?.finish() }
        })
        navigationItem.rightBarButtonItem?.accessibilityIdentifier = "claude-config-close"
        for (i, info) in panel.tabs().enumerated() {
            tabs.insertSegment(withTitle: info.label, at: i, animated: false)
        }
        tabs.selectedSegmentIndex = panel.tabs().firstIndex { $0.tab == current } ?? 1
        search.text = query
        tabs.accessibilityIdentifier = "claude-config-tabs"
        tabs.addAction(UIAction { [weak self] _ in self?.selectTab() }, for: .valueChanged)
        search.placeholder = "Search settings…"
        search.delegate = self
        search.accessibilityIdentifier = "claude-config-search"
        search.returnKeyType = .search
        search.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            self.query = self.search.text ?? ""
            self.expanded = nil
            self.reload()
        }, for: .editingChanged)
        chips.axis = .horizontal
        chips.spacing = 6
        shown.font = Fonts.ui(.sans, 12)
        shown.textColor = Palette.tertiary
        footer.font = Fonts.ui(.sans, 12)
        footer.textColor = Palette.secondary
        footer.numberOfLines = 0
        footer.accessibilityIdentifier = "claude-config-footer"
        reload()
        Task { @MainActor [weak self] in
            await self?.panel.load()
            self?.reload()
        }
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        finish()
    }

    private func finish() {
        guard !finished else { return }
        finished = true
        let panel = panel, failed = onCloseFailed
        Task { @MainActor in
            do {
                _ = try await panel.close()
            } catch {
                failed?("\(error)")
            }
        }
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        textField.resignFirstResponder()
        return true
    }

    private func selectTab() {
        current = panel.tabs()[tabs.selectedSegmentIndex].tab
        expanded = nil
        search.resignFirstResponder()
        if current == .stats, !statsRequested {
            statsRequested = true
            Task { @MainActor [weak self] in
                await self?.panel.loadStats()
                self?.reload()
            }
        }
        tableView.tableHeaderView = header()
        reload()
    }

    private func reload() {
        list = panel.list(filter: filter, query: query)
        footer.text = list.footer
        shown.text = list.shown
        rebuildChips()
        tableView.reloadData()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        if tableView.tableHeaderView == nil || tableView.tableHeaderView?.frame.width != tableView.bounds.width {
            tableView.tableHeaderView = header()
        }
    }

    private func header() -> UIView {
        let stack = UIStackView(arrangedSubviews: [tabs])
        stack.axis = .vertical
        stack.spacing = 10
        if current == .config {
            let scroll = UIScrollView()
            scroll.showsHorizontalScrollIndicator = false
            let content = UIStackView(arrangedSubviews: [chips, shown])
            content.alignment = .center
            content.spacing = 10
            content.translatesAutoresizingMaskIntoConstraints = false
            scroll.addSubview(content)
            NSLayoutConstraint.activate([
                content.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
                content.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor),
                content.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
                content.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
                content.heightAnchor.constraint(equalTo: scroll.frameLayoutGuide.heightAnchor),
                scroll.heightAnchor.constraint(equalToConstant: 32),
            ])
            stack.addArrangedSubview(search)
            stack.addArrangedSubview(scroll)
        }
        let wrap = UIView()
        stack.translatesAutoresizingMaskIntoConstraints = false
        wrap.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: wrap.topAnchor, constant: 8),
            stack.leadingAnchor.constraint(equalTo: wrap.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: wrap.trailingAnchor, constant: -20),
            stack.bottomAnchor.constraint(equalTo: wrap.bottomAnchor, constant: -4),
            search.heightAnchor.constraint(equalToConstant: 36),
        ])
        let width = tableView.bounds.width > 0 ? tableView.bounds.width : view.bounds.width
        wrap.frame.size = wrap.systemLayoutSizeFitting(CGSize(width: width, height: 0), withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
        return wrap
    }

    private func rebuildChips() {
        chips.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for chip in panel.filters() {
            let on = chip.filter == filter
            var c = UIButton.Configuration.filled()
            c.cornerStyle = .capsule
            c.contentInsets = NSDirectionalEdgeInsets(top: 5, leading: 10, bottom: 5, trailing: 10)
            var t = AttributedString("\(chip.label) \(chip.count)")
            t.font = Fonts.ui(.sansMedium, 12.5)
            c.attributedTitle = t
            c.titleLineBreakMode = .byClipping
            c.baseForegroundColor = on ? Palette.accent : Palette.secondary
            c.baseBackgroundColor = on ? Palette.accentSoft : Palette.controlFill
            let b = UIButton(configuration: c, primaryAction: UIAction { [weak self] _ in
                self?.filter = chip.filter
                self?.expanded = nil
                self?.reload()
            })
            b.accessibilityIdentifier = "claude-config-filter-\(chip.label)"
            b.accessibilityTraits = on ? [.button, .selected] : .button
            chips.addArrangedSubview(b)
        }
    }

    private enum Line {
        case setting(ConfigRowView)
        case option(ConfigRowView, String)
    }

    private var lines: [Line] {
        list.rows.flatMap { row -> [Line] in
            guard expanded == row.id else { return [.setting(row)] }
            return [.setting(row)] + row.options.map { .option(row, $0) }
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { 1 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        switch current {
        case .config: list.message == nil ? lines.count : 1
        case .status: panel.status().rows.count
        case .usage: usageLines.count
        case .stats: statsLines.count
        }
    }

    override func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        switch current {
        case .config: list.footer
        case .status: panel.status().footnote
        case .usage: nil
        case .stats: panel.stats().footnote
        }
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        switch current {
        case .config:
            if let message = list.message { return messageCell(message) }
            switch lines[indexPath.row] {
            case let .setting(row): return settingCell(row)
            case let .option(row, option): return optionCell(row, option)
            }
        case .status:
            let row = panel.status().rows[indexPath.row]
            let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
            var c = UIListContentConfiguration.valueCell()
            c.prefersSideBySideTextAndSecondaryText = false
            c.text = row.key
            c.textProperties.color = Palette.secondary
            c.textProperties.font = Fonts.ui(.sans, 13)
            c.secondaryText = row.value
            c.secondaryTextProperties.font = Fonts.ui(.mono, 13)
            c.secondaryTextProperties.color = Palette.text
            cell.contentConfiguration = c
            cell.accessoryView = row.chip ? tag("= chip", chip: true) : nil
            cell.selectionStyle = .none
            cell.accessibilityIdentifier = "claude-config-status-\(row.key)"
            return cell
        case .usage: return usageLines[indexPath.row]()
        case .stats: return statsLines[indexPath.row]()
        }
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        guard current == .config, list.message == nil else { return }
        switch lines[indexPath.row] {
        case let .setting(row): activate(row)
        case let .option(row, option):
            expanded = nil
            change(row, to: option)
        }
    }

    private func activate(_ row: ConfigRowView) {
        selected = row.id
        guard row.writable else { return reload() }
        switch row.kind {
        case .toggle, .cycle:
            if let next = row.nextValue { change(row, to: next) }
        case .options:
            expanded = expanded == row.id ? nil : row.id
            reload()
        case .text:
            let alert = UIAlertController(title: row.label, message: row.key, preferredStyle: .alert)
            alert.addTextField { $0.text = row.value; $0.placeholder = "e.g. Vietnamese" }
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            alert.addAction(UIAlertAction(title: "Save", style: .default) { [weak self] _ in
                self?.change(row, to: alert.textFields?.first?.text?.trimmingCharacters(in: .whitespaces) ?? "")
            })
            present(alert, animated: true)
        }
    }

    private func change(_ row: ConfigRowView, to value: String) {
        UISelectionFeedbackGenerator().selectionChanged()
        Task { @MainActor [weak self] in
            guard let self else { return }
            let pending = Task { @MainActor [weak self] in
                try? await Task.sleep(for: .milliseconds(30))
                self?.reload()
            }
            let chip = await self.panel.change(id: row.id, value: value)
            pending.cancel()
            if let chip {
                await self.applyChip?(chip)
                await self.panel.refreshChips()
            }
            self.reload()
        }
    }

    private func messageCell(_ text: String) -> UITableViewCell {
        let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
        var c = cell.defaultContentConfiguration()
        c.text = text
        c.textProperties.color = Palette.tertiary
        c.textProperties.alignment = .center
        c.textProperties.font = Fonts.ui(.sans, 14)
        cell.contentConfiguration = c
        cell.selectionStyle = .none
        cell.accessibilityIdentifier = "claude-config-message"
        return cell
    }

    private func settingCell(_ row: ConfigRowView) -> UITableViewCell {
        let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
        var c = cell.defaultContentConfiguration()
        c.attributedText = highlighted(row.label, font: Fonts.ui(.sansMedium, 15), color: row.writable ? Palette.text : Palette.tertiary)
        let key = highlighted(row.key, font: Fonts.ui(.mono, 11.5), color: Palette.tertiary)
        if let note = row.note, selected == row.id || row.notePinned {
            key.append(NSAttributedString(string: "\n" + note, attributes: [.font: Fonts.ui(.sans, 12), .foregroundColor: Palette.secondary]))
        }
        c.secondaryAttributedText = key
        c.textToSecondaryTextVerticalPadding = 2
        cell.contentConfiguration = c
        let accessory = UIStackView()
        accessory.axis = .horizontal
        accessory.alignment = .center
        accessory.spacing = 6
        if row.changed {
            let dot = UIView()
            dot.backgroundColor = Palette.accent
            dot.layer.cornerRadius = 3
            dot.widthAnchor.constraint(equalToConstant: 6).isActive = true
            dot.heightAnchor.constraint(equalToConstant: 6).isActive = true
            accessory.addArrangedSubview(dot)
        }
        if row.kind == .toggle {
            let toggle = UISwitch()
            toggle.isOn = row.value == "true"
            toggle.isEnabled = row.writable
            toggle.accessibilityIdentifier = "claude-config-toggle-\(row.id)"
            toggle.addAction(UIAction { [weak self] _ in self?.activate(row) }, for: .valueChanged)
            accessory.addArrangedSubview(toggle)
        } else {
            let value = UILabel()
            value.attributedText = highlighted(row.value.isEmpty ? "default" : row.value, font: Fonts.ui(.mono, 13), color: row.value.isEmpty ? Palette.tertiary : (row.writable ? Palette.text : Palette.tertiary))
            value.lineBreakMode = .byTruncatingTail
            value.widthAnchor.constraint(lessThanOrEqualToConstant: 150).isActive = true
            accessory.addArrangedSubview(value)
            if row.kind == .options, row.writable {
                let chevron = UIImageView(image: UIImage(systemName: expanded == row.id ? "chevron.up" : "chevron.down", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11, weight: .semibold)))
                chevron.tintColor = Palette.tertiary
                accessory.addArrangedSubview(chevron)
            }
        }
        if let label = row.tag {
            accessory.addArrangedSubview(tag(label, chip: row.chip))
        }
        accessory.frame.size = accessory.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize)
        cell.accessoryView = accessory
        cell.accessibilityIdentifier = "claude-config-row-\(row.id)"
        cell.backgroundColor = selected == row.id ? Palette.accentSoft : nil
        return cell
    }

    private func optionCell(_ row: ConfigRowView, _ option: String) -> UITableViewCell {
        let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
        var c = cell.defaultContentConfiguration()
        c.text = option
        c.textProperties.font = Fonts.ui(.mono, 13.5)
        c.directionalLayoutMargins.leading = 32
        cell.contentConfiguration = c
        cell.accessoryType = option == row.value ? .checkmark : .none
        cell.tintColor = Palette.accent
        cell.accessibilityIdentifier = "claude-config-option-\(option)"
        return cell
    }

    private func highlighted(_ text: String, font: UIFont, color: UIColor) -> NSMutableAttributedString {
        let out = NSMutableAttributedString(string: text, attributes: [.font: font, .foregroundColor: color])
        let q = query.trimmingCharacters(in: .whitespaces)
        if !q.isEmpty {
            let r = (text as NSString).range(of: q, options: .caseInsensitive)
            if r.location != NSNotFound {
                out.addAttribute(.backgroundColor, value: Palette.warning.withAlphaComponent(0.3), range: r)
            }
        }
        return out
    }

    private func tag(_ text: String, chip: Bool) -> UIView {
        let label = PaddedLabel(insets: UIEdgeInsets(top: 2, left: 6, bottom: 2, right: 6))
        label.text = text
        label.font = Fonts.ui(.sansMedium, 11)
        label.textColor = chip ? Palette.accent : Palette.secondary
        label.backgroundColor = chip ? Palette.accentSoft : Palette.controlFill
        label.layer.cornerRadius = 5
        label.clipsToBounds = true
        label.frame.size = label.intrinsicContentSize
        return label
    }

    private var usageLines: [() -> UITableViewCell] {
        let report = panel.usage()
        if let message = report.message { return [{ [unowned self] in self.messageCell(message) }] }
        var out: [() -> UITableViewCell] = report.meters.map { meter in { MeterCell(meter: meter) } }
        for section in report.sections {
            out.append { [unowned self] in self.textCell(section.title, lines: section.lines) }
        }
        if let note = report.note {
            out.append { [unowned self] in self.textCell(note, lines: []) }
        }
        out.append { [unowned self] in
            let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
            var c = UIButton.Configuration.bordered()
            c.title = "Open Usage page"
            c.cornerStyle = .capsule
            let b = UIButton(configuration: c, primaryAction: UIAction { [weak self] _ in
                self?.dismiss(animated: true) {
                    self?.finish()
                    self?.openUsage?()
                }
            })
            b.accessibilityIdentifier = "claude-config-open-usage"
            b.translatesAutoresizingMaskIntoConstraints = false
            cell.contentView.addSubview(b)
            NSLayoutConstraint.activate([
                b.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
                b.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 8),
                b.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -8),
            ])
            cell.selectionStyle = .none
            return cell
        }
        return out
    }

    private func textCell(_ title: String, lines: [String]) -> UITableViewCell {
        let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
        var c = cell.defaultContentConfiguration()
        c.text = title
        c.textProperties.font = Fonts.ui(.sansMedium, 14)
        if !lines.isEmpty {
            c.secondaryText = lines.joined(separator: "\n")
            c.secondaryTextProperties.font = Fonts.ui(.sans, 12.5)
            c.secondaryTextProperties.color = Palette.secondary
        }
        cell.contentConfiguration = c
        cell.selectionStyle = .none
        return cell
    }

    private var statsLines: [() -> UITableViewCell] {
        let stats = panel.stats()
        if let message = stats.message { return [{ [unowned self] in self.messageCell(message) }] }
        return [{ HeatCell(caption: stats.caption, heat: [UInt8](stats.heat)) }, { TilesCell(tiles: stats.tiles) }]
    }
}

private final class MeterCell: UITableViewCell {
    init(meter: UsageMeterView) {
        super.init(style: .default, reuseIdentifier: nil)
        selectionStyle = .none
        let label = UILabel()
        label.text = meter.label
        label.font = Fonts.ui(.sans, 14)
        let value = UILabel()
        value.text = "\(meter.percent)%"
        value.font = Fonts.ui(.sansMedium, 14)
        value.textColor = meter.warning ? Palette.warning : Palette.secondary
        let top = UIStackView(arrangedSubviews: [label, UIView(), value])
        let track = UIView()
        track.backgroundColor = Palette.controlFill
        track.layer.cornerRadius = 3
        let bar = UIView()
        bar.backgroundColor = meter.warning ? Palette.warning : Palette.accent
        bar.layer.cornerRadius = 3
        bar.translatesAutoresizingMaskIntoConstraints = false
        track.addSubview(bar)
        let foot = UILabel()
        foot.text = meter.resets
        foot.font = Fonts.ui(.sans, 12)
        foot.textColor = Palette.tertiary
        foot.isHidden = meter.resets == nil
        let stack = UIStackView(arrangedSubviews: [top, track, foot])
        stack.axis = .vertical
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)
        let fraction = CGFloat(min(meter.percent, 100)) / 100
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: contentView.layoutMarginsGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: contentView.layoutMarginsGuide.trailingAnchor),
            stack.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -10),
            track.heightAnchor.constraint(equalToConstant: 6),
            bar.leadingAnchor.constraint(equalTo: track.leadingAnchor),
            bar.topAnchor.constraint(equalTo: track.topAnchor),
            bar.bottomAnchor.constraint(equalTo: track.bottomAnchor),
            bar.widthAnchor.constraint(equalTo: track.widthAnchor, multiplier: max(fraction, 0.02)),
        ])
        accessibilityIdentifier = "claude-config-meter-\(meter.label)"
    }

    required init?(coder: NSCoder) { fatalError() }
}

private final class HeatCell: UITableViewCell {
    init(caption: String, heat: [UInt8]) {
        super.init(style: .default, reuseIdentifier: nil)
        selectionStyle = .none
        let title = UILabel()
        title.text = caption
        title.font = Fonts.ui(.sans, 12)
        title.textColor = Palette.tertiary
        let grid = UIStackView()
        grid.axis = .horizontal
        grid.spacing = 2
        grid.alignment = .top
        for start in stride(from: 0, to: heat.count, by: 7) {
            let week = UIStackView()
            week.axis = .vertical
            week.spacing = 2
            for level in heat[start..<min(start + 7, heat.count)] {
                let cell = UIView()
                cell.layer.cornerRadius = 1.5
                cell.widthAnchor.constraint(equalToConstant: 7).isActive = true
                cell.heightAnchor.constraint(equalToConstant: 7).isActive = true
                cell.backgroundColor = level == 0 ? Palette.controlFill : Palette.accent.withAlphaComponent([0, 0.25, 0.45, 0.7, 1][Int(min(level, 4))])
                week.addArrangedSubview(cell)
            }
            grid.addArrangedSubview(week)
        }
        let scroll = UIScrollView()
        scroll.showsHorizontalScrollIndicator = false
        grid.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(grid)
        NSLayoutConstraint.activate([
            grid.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            grid.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor),
            grid.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            grid.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
            grid.heightAnchor.constraint(equalTo: scroll.frameLayoutGuide.heightAnchor),
        ])
        let stack = UIStackView(arrangedSubviews: [title, scroll])
        stack.axis = .vertical
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: contentView.layoutMarginsGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: contentView.layoutMarginsGuide.trailingAnchor),
            stack.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -10),
            scroll.heightAnchor.constraint(equalToConstant: 7 * 7 + 6 * 2),
        ])
        accessibilityIdentifier = "claude-config-heat"
    }

    required init?(coder: NSCoder) { fatalError() }
}

private final class TilesCell: UITableViewCell {
    init(tiles: [StatTile]) {
        super.init(style: .default, reuseIdentifier: nil)
        selectionStyle = .none
        let rows = UIStackView()
        rows.axis = .vertical
        rows.spacing = 8
        for start in stride(from: 0, to: tiles.count, by: 2) {
            let row = UIStackView()
            row.spacing = 8
            row.distribution = .fillEqually
            for tile in tiles[start..<min(start + 2, tiles.count)] {
                let value = UILabel()
                value.text = tile.value
                value.font = Fonts.ui(.sansMedium, 15)
                value.adjustsFontSizeToFitWidth = true
                let label = UILabel()
                label.text = tile.label
                label.font = Fonts.ui(.sans, 12)
                label.textColor = Palette.tertiary
                let box = UIStackView(arrangedSubviews: [value, label])
                box.axis = .vertical
                box.spacing = 2
                box.isLayoutMarginsRelativeArrangement = true
                box.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 8, leading: 10, bottom: 8, trailing: 10)
                box.layer.cornerRadius = 10
                box.layer.borderWidth = 1
                box.layer.borderColor = Palette.controlFill.cgColor
                row.addArrangedSubview(box)
            }
            rows.addArrangedSubview(row)
        }
        rows.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(rows)
        NSLayoutConstraint.activate([
            rows.leadingAnchor.constraint(equalTo: contentView.layoutMarginsGuide.leadingAnchor),
            rows.trailingAnchor.constraint(equalTo: contentView.layoutMarginsGuide.trailingAnchor),
            rows.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10),
            rows.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -10),
        ])
        accessibilityIdentifier = "claude-config-tiles"
    }

    required init?(coder: NSCoder) { fatalError() }
}
