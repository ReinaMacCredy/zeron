import UIKit

struct SettingsRow: Hashable {
    let id: String
    let title: String
    var subtitle: String?
    var symbol: String
    var tint: UIColor = Palette.secondary
    var accessory: Accessory = .disclosure
    var destructive = false
    var disabled = false

    enum Accessory: Hashable {
        case disclosure
        case none
        case check(Bool)
        case dot(Bool)
        case toggle(Bool)
        case segmented([String], Int)
        case slider(Slider)
    }

    struct Slider: Hashable {
        var value: Double
        var max: Double
        var step: Double
        var scale: Double
        var unit: String

        func text(_ v: Double) -> String { "\(Int((v * scale).rounded()))\(unit)" }
    }

    struct Handlers {
        var toggled: (String, Bool) -> Void = { _, _ in }
        var picked: (String, Int) -> Void = { _, _ in }
        var slid: (String, Double) -> Void = { _, _ in }
    }

    static func registration(_ handlers: @escaping () -> Handlers?) -> UICollectionView.CellRegistration<UICollectionViewListCell, SettingsRow> {
        UICollectionView.CellRegistration { cell, _, row in
            var c = UIListContentConfiguration.subtitleCell()
            c.text = row.title
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            c.textProperties.color = row.destructive ? Palette.danger : row.disabled ? Palette.tertiary : Palette.text
            c.secondaryText = row.subtitle
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.secondaryTextProperties.color = Palette.secondary
            c.image = UIImage(systemName: row.symbol)
            c.imageProperties.tintColor = row.destructive ? Palette.danger : row.tint
            cell.contentConfiguration = c
            var bg = UIBackgroundConfiguration.listGroupedCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
            switch row.accessory {
            case .disclosure: cell.accessories = [.disclosureIndicator()]
            case .none: cell.accessories = []
            case let .check(on): cell.accessories = on ? [.checkmark(options: .init(tintColor: Palette.accent))] : []
            case let .dot(online):
                let dot = UIView(frame: CGRect(x: 0, y: 0, width: 8, height: 8))
                dot.backgroundColor = online ? Palette.success : Palette.tertiary
                dot.layer.cornerRadius = 4
                cell.accessories = [.customView(configuration: .init(customView: dot, placement: .trailing()))]
            case let .toggle(on):
                let toggle = UISwitch()
                toggle.isOn = on
                toggle.onTintColor = Palette.accent
                toggle.isEnabled = !row.disabled
                toggle.accessibilityIdentifier = "toggle-\(row.id)"
                toggle.addAction(UIAction { [weak toggle] _ in
                    handlers()?.toggled(row.id, toggle?.isOn ?? false)
                }, for: .valueChanged)
                cell.accessories = [.customView(configuration: .init(customView: toggle, placement: .trailing()))]
            case let .segmented(items, selected):
                let control = UISegmentedControl(items: items)
                control.selectedSegmentIndex = selected
                control.isEnabled = !row.disabled
                control.accessibilityIdentifier = "segment-\(row.id)"
                control.addAction(UIAction { [weak control] _ in
                    handlers()?.picked(row.id, control?.selectedSegmentIndex ?? 0)
                }, for: .valueChanged)
                control.sizeToFit()
                cell.accessories = [.customView(configuration: .init(customView: control, placement: .trailing(), reservedLayoutWidth: .actual, maintainsFixedSize: true))]
            case let .slider(spec):
                let slider = UISlider()
                slider.maximumValue = Float(spec.max)
                slider.value = Float(spec.value)
                slider.minimumTrackTintColor = Palette.accent
                slider.isEnabled = !row.disabled
                slider.accessibilityIdentifier = "slider-\(row.id)"
                let label = UILabel()
                label.font = Fonts.ui(.sans, 13)
                label.textColor = Palette.secondary
                label.textAlignment = .right
                label.text = spec.text(spec.value)
                label.accessibilityIdentifier = "value-\(row.id)"
                slider.addAction(UIAction { [weak slider, weak label] _ in
                    guard let slider else { return }
                    let value = (Double(slider.value) / spec.step).rounded() * spec.step
                    label?.text = spec.text(value)
                    handlers()?.slid(row.id, value)
                }, for: .valueChanged)
                let stack = UIStackView(arrangedSubviews: [slider, label])
                stack.spacing = 8
                stack.frame = CGRect(x: 0, y: 0, width: 168, height: 32)
                label.widthAnchor.constraint(equalToConstant: 42).isActive = true
                cell.accessories = [.customView(configuration: .init(customView: stack, placement: .trailing(), reservedLayoutWidth: .actual, maintainsFixedSize: true))]
            }
            cell.accessibilityIdentifier = "settings-\(row.id)"
        }
    }
}
