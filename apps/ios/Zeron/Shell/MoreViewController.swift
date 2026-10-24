import UIKit
import UserNotifications

/// Settings & utilities: account, providers, Cloud and billing, devices,
/// appearance, archive.
final class MoreViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!

    struct Row: Hashable {
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
    }

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
        title = "Settings"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .always
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        view.addSubview(collectionView)

        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { [weak self] cell, _, row in
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
                toggle.addAction(UIAction { [weak self, weak toggle] _ in
                    self?.toggled(row.id, toggle?.isOn ?? false)
                }, for: .valueChanged)
                cell.accessories = [.customView(configuration: .init(customView: toggle, placement: .trailing()))]
            case let .segmented(items, selected):
                let control = UISegmentedControl(items: items)
                control.selectedSegmentIndex = selected
                control.isEnabled = !row.disabled
                control.accessibilityIdentifier = "segment-\(row.id)"
                control.addAction(UIAction { [weak self, weak control] _ in
                    self?.picked(row.id, control?.selectedSegmentIndex ?? 0)
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
                slider.addAction(UIAction { [weak self, weak slider, weak label] _ in
                    guard let slider else { return }
                    let value = (Double(slider.value) / spec.step).rounded() * spec.step
                    label?.text = spec.text(value)
                    self?.slid(row.id, value)
                }, for: .valueChanged)
                let stack = UIStackView(arrangedSubviews: [slider, label])
                stack.spacing = 8
                stack.frame = CGRect(x: 0, y: 0, width: 168, height: 32)
                label.widthAnchor.constraint(equalToConstant: 42).isActive = true
                cell.accessories = [.customView(configuration: .init(customView: stack, placement: .trailing(), reservedLayoutWidth: .actual, maintainsFixedSize: true))]
            }
            cell.accessibilityIdentifier = "settings-\(row.id)"
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionHeader) { [weak self] view, _, path in
            var c = UIListContentConfiguration.groupedHeader()
            c.text = self?.dataSource.snapshot().sectionIdentifiers[path.section]
            view.contentConfiguration = c
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, row in
            cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        dataSource.supplementaryViewProvider = { cv, _, path in
            cv.dequeueConfiguredReusableSupplementary(using: header, for: path)
        }
        reload()
        wallpaperObserver = NotificationCenter.default.addObserver(forName: WallpaperStore.didChange, object: nil, queue: .main) { [weak self] _ in
            if self?.sliding == false { self?.reload() }
        }
        // Devices go on/offline and the org name backfills after opening.
        appToken = app.observe { [weak self] in self?.reload() }
        cloudToken = app.cloud.observe { [weak self] in self?.reload() }
        app.cloud.refresh()
        // Back from iOS Settings (notifications may have been allowed there).
        foregroundObserver = NotificationCenter.default.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { [weak self] _ in
            self?.refreshNotificationStatus()
        }
    }

    private var foregroundObserver: NSObjectProtocol?

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        if dataSource != nil { reload() }
        refreshNotificationStatus()
        WallpaperSync.refresh(app)
    }

    // MARK: Notifications

    private var notificationStatus: UNAuthorizationStatus = .notDetermined

    private func refreshNotificationStatus() {
        Task { @MainActor in
            let status = await PushNotifications.shared.authorizationStatus()
            guard status != self.notificationStatus else { return }
            self.notificationStatus = status
            self.reload()
        }
    }

    /// On when this phone wants them and iOS allows them.
    private var notificationsOn: Bool {
        PushNotifications.shared.enabled && (notificationStatus == .authorized || notificationStatus == .provisional || notificationStatus == .ephemeral)
    }

    private func notificationRows() -> [Row] {
        var rows = [Row(id: "notify:enabled", title: "Notifications", subtitle: notificationStatus == .denied ? "Turned off in iOS Settings" : "When a session finishes, needs you or fails", symbol: "bell", accessory: .toggle(notificationsOn))]
        if notificationsOn {
            rows += PushNotifications.Kind.allCases.map { kind in
                let symbol: String = switch kind {
                case .done: "checkmark.circle"
                case .input: "questionmark.bubble"
                case .failed: "exclamationmark.triangle"
                }
                return Row(id: "notify:\(kind.rawValue)", title: kind.title, symbol: symbol, accessory: .toggle(PushNotifications.shared.isOn(kind)))
            }
        }
        return rows
    }

    private func toggled(_ id: String, _ on: Bool) {
        if id == "groupByProject" {
            app.groupByProject = on
            return
        }
        if id == "wallpaper:sync" {
            WallpaperStore.syncWithMac = on
            WallpaperSync.refresh(app)
            return
        }
        if id == "wallpaper:chats" || id == "wallpaper:scrim" {
            var chat = WallpaperStore.chatBackground
            if id == "wallpaper:chats" { chat.enabled = on } else { chat.scrim = on }
            WallpaperStore.chatBackground = chat
            return
        }
        if id == "math:render" {
            MathSettings.render = on
            return reload()
        }
        let push = PushNotifications.shared
        if id == "notify:enabled" {
            guard on else {
                push.enabled = false
                return reload()
            }
            if notificationStatus == .denied {
                // Only the Settings app can turn them back on.
                let alert = UIAlertController(title: "Notifications are off", message: "Allow notifications for Zeron in iOS Settings.", preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "Not Now", style: .cancel) { [weak self] _ in self?.reload() })
                alert.addAction(UIAlertAction(title: "Open Settings", style: .default) { [weak self] _ in
                    if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                    self?.reload()
                })
                return present(alert, animated: true)
            }
            push.enabled = true
            Task { @MainActor in
                await push.requestPermission()
                self.notificationStatus = await push.authorizationStatus()
                self.reload()
            }
            return
        }
        if let kind = PushNotifications.Kind(rawValue: String(id.dropFirst("notify:".count))) {
            push.set(kind, on)
        }
    }

    private func picked(_ id: String, _ index: Int) {
        guard id == "wallpaper:placement" else { return }
        var chat = WallpaperStore.chatBackground
        chat.placement = WallpaperStore.ChatBackground.Placement.allCases[index]
        WallpaperStore.chatBackground = chat
    }

    // Rebuilding the rows mid-drag would replace the slider under the finger.
    private var sliding = false

    private func slid(_ id: String, _ value: Double) {
        var chat = WallpaperStore.chatBackground
        if id == "wallpaper:opacity" { chat.opacity = CGFloat(value) } else { chat.blur = CGFloat(value) }
        sliding = true
        WallpaperStore.chatBackground = chat
        sliding = false
    }

    private func wallpaperRows() -> [Row] {
        let syncing = WallpaperStore.syncWithMac
        let host = WallpaperSync.host(app)
        let follows = host.map { syncing ? "Follows \($0.name)\($0.online ? "" : " (offline)")" : "Use \($0.name)'s background and chat settings" }
        var rows = [Row(id: "wallpaper:sync", title: "Sync with Mac", subtitle: follows ?? "No Mac in this workspace", symbol: "laptopcomputer.and.iphone", accessory: .toggle(syncing), disabled: host == nil && !syncing)]
        if syncing, (app.client?.wallpaperHosts().count ?? 0) > 1 {
            rows.append(Row(id: "wallpaper:host", title: "Mac", subtitle: host?.name, symbol: "desktopcomputer"))
        }
        let set = WallpaperStore.isSet
        let choose = syncing ? "Wallpaper" : set ? "Change Wallpaper…" : "Choose Wallpaper…"
        rows.append(Row(id: "wallpaper:choose", title: choose, subtitle: set ? WallpaperStore.name : syncing ? "None on the Mac" : "Shown behind new chats and the sessions list", symbol: "photo", accessory: syncing ? .none : .disclosure, disabled: syncing))
        guard set else { return rows }
        let effect = WallpaperStore.effect
        rows.append(Row(id: "wallpaper:effect", title: "Effect", subtitle: "\(WallpaperStore.label(effect)) — \(WallpaperStore.detail(effect))", symbol: "wand.and.stars", accessory: syncing ? .none : .disclosure, disabled: syncing))
        let chat = WallpaperStore.chatBackground
        rows.append(Row(id: "wallpaper:chats", title: "Chat background", subtitle: "Show the artwork behind open conversations.", symbol: "bubble.left.and.text.bubble.right", accessory: .toggle(chat.enabled), disabled: syncing))
        if chat.enabled {
            let placements = WallpaperStore.ChatBackground.Placement.allCases
            rows.append(Row(id: "wallpaper:placement", title: "Placement", subtitle: chat.placement.detail, symbol: "rectangle.portrait.topthird.inset.filled", accessory: .segmented(placements.map(\.label), placements.firstIndex(of: chat.placement) ?? 0), disabled: syncing))
            rows.append(Row(id: "wallpaper:opacity", title: "Opacity", symbol: "circle.lefthalf.filled", accessory: .slider(.init(value: Double(chat.opacity), max: 1, step: 0.01, scale: 100, unit: "%")), disabled: syncing))
            rows.append(Row(id: "wallpaper:blur", title: "Blur", symbol: "drop", accessory: .slider(.init(value: Double(chat.blur), max: Double(WallpaperStore.ChatBackground.blurMax), step: Double(WallpaperStore.ChatBackground.blurStep), scale: 1, unit: "px")), disabled: syncing))
            rows.append(Row(id: "wallpaper:scrim", title: "Scrim", subtitle: "Darkens the artwork toward the bottom so text stays readable.", symbol: "square.bottomhalf.filled", accessory: .toggle(chat.scrim), disabled: syncing))
        }
        if !syncing {
            rows.append(Row(id: "wallpaper:remove", title: "Remove Wallpaper", symbol: "trash", accessory: .none, destructive: true))
        }
        return rows
    }

    private var wallpaperObserver: NSObjectProtocol?
    private var appToken: AnyObject?
    private var cloudToken: AnyObject?

    private func reload() {
        var s = NSDiffableDataSourceSnapshot<String, Row>()
        s.appendSections(["Account"])
        s.appendItems([
            Row(id: "account", title: app.accountName, subtitle: app.accountDetail, symbol: "person.crop.circle", accessory: .none),
        ])
        // Agents and where they run, as the desktop groups them: Providers,
        // then Cloud and Billing when this account is offered Cloud.
        s.appendSections(["Agents"])
        var agents = [Row(id: "providers", title: "Providers", subtitle: "Agents and their accounts on each device", symbol: "cpu")]
        let demoCloud = app.isDemo && ProcessInfo.processInfo.arguments.contains("-cloud")
        if (!app.isDemo && app.cloud.available) || demoCloud {
            let cloud = app.cloud
            let github = cloud.githubAccount.map { "GitHub @\($0)" } ?? "Connect GitHub"
            agents.append(Row(id: "cloud", title: "Cloud", subtitle: cloud.problem?.title ?? github, symbol: "cloud"))
            let credits = cloud.credits.map { "\(CloudUI.credits($0)) credits" }
            let plan = cloud.billing?.plan.map { "\($0.name) plan" }
            agents.append(Row(id: "billing", title: "Billing", subtitle: [plan, credits].compactMap { $0 }.joined(separator: " · ").nonEmpty ?? "Plans and credits", symbol: "creditcard"))
        }
        s.appendItems(agents)
        s.appendSections(["Devices"])
        s.appendItems(app.hostOptions.map { Row(id: "device:\($0.id)", title: $0.name, subtitle: $0.online ? "Online" : "Offline", symbol: "desktopcomputer", accessory: .dot($0.online)) })
        if app.voice.available {
            s.appendSections(["Voice"])
            s.appendItems([Row(id: "voice", title: "Voice", subtitle: app.voice.selectedStyle.map { "Codex voice · \($0.capitalized)" } ?? "Codex voice device and style", symbol: "waveform")])
        }
        s.appendSections(["Notifications"])
        s.appendItems(notificationRows())
        s.appendSections(["Appearance"])
        let style = UserDefaults.standard.integer(forKey: "appearance")
        s.appendItems([
            Row(id: "appearance:0", title: "System", symbol: "circle.lefthalf.filled", accessory: .check(style == 0)),
            Row(id: "appearance:1", title: "Light", symbol: "sun.max", accessory: .check(style == 1)),
            Row(id: "appearance:2", title: "Dark", symbol: "moon", accessory: .check(style == 2)),
        ])
        s.appendSections(["Wallpaper"])
        s.appendItems(wallpaperRows())
        s.appendSections(["Math"])
        var math = [Row(id: "math:render", title: "Render Math", subtitle: "Off shows LaTeX as its source", symbol: "function", accessory: .toggle(MathSettings.render))]
        if MathSettings.render {
            let face = MathSettings.face
            math.append(Row(id: "math:font", title: "Math Font", subtitle: "\(MathSettings.label(face)) — \(MathSettings.detail(face))", symbol: "textformat"))
            math.append(Row(id: "math:size", title: "Math Size", subtitle: "\(MathSettings.size)% of the text around it", symbol: "textformat.size"))
        }
        s.appendItems(math)
        s.appendSections(["Sessions"])
        s.appendItems([
            Row(id: "groupByProject", title: "Group by Project", subtitle: "Sessions outside your sections, under their project", symbol: "square.stack.3d.up", accessory: .toggle(app.groupByProject)),
            Row(id: "archived", title: "Archived Sessions", symbol: "archivebox"),
        ])
        s.appendSections(["Composer"])
        let steer = UserDefaults.standard.string(forKey: "followUpBehavior") == "steer"
        s.appendItems([
            Row(id: "followUp:queue", title: "Queue", subtitle: "Queue follow-ups while the agent runs", symbol: "text.line.last.and.arrowtriangle.forward", accessory: .check(!steer)),
            Row(id: "followUp:steer", title: "Steer", subtitle: "Steer the current run", symbol: "arrow.triangle.turn.up.right.diamond", accessory: .check(steer)),
        ])
        let mission = chatMissionLabels()
        s.appendSections([mission.title])
        s.appendItems([
            Row(id: "chatMission:lead", title: mission.lead, subtitle: mission.detail, symbol: "target", accessory: .check(ChatMission.lead)),
            Row(id: "chatMission:newMission", title: mission.newMission, symbol: "plus.bubble", accessory: .check(!ChatMission.lead)),
        ])
        s.appendSections(["Help"])
        s.appendItems([
            Row(id: "whatsnew", title: "What's New", subtitle: whatsNew().map { "v\($0.version) · \($0.date)" }, symbol: "sparkles"),
            Row(id: "docs", title: "Docs", subtitle: "How Zeron works, by topic", symbol: "book"),
        ])
        s.appendSections([" "])
        s.appendItems([Row(id: "signout", title: "Sign Out", symbol: "rectangle.portrait.and.arrow.right", accessory: .none, destructive: true)])
        dataSource.apply(s, animatingDifferences: false)
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt path: IndexPath) {
        collectionView.deselectItem(at: path, animated: true)
        guard let row = dataSource.itemIdentifier(for: path), !row.disabled else { return }
        switch row.id {
        case "voice": navigationController?.pushViewController(VoiceViewController(app: app), animated: true)
        case "providers":
            navigationController?.pushViewController(ProvidersViewController(app: app), animated: true)
        case "cloud":
            navigationController?.pushViewController(CloudSettingsViewController(app: app), animated: true)
        case "billing":
            navigationController?.pushViewController(BillingViewController(app: app), animated: true)
        case let id where id.hasPrefix("device:"):
            navigationController?.pushViewController(HostPageViewController(app: app, deviceId: String(id.dropFirst(7)), page: "mac", title: row.title), animated: true)
        case let id where id.hasPrefix("followUp:"):
            UserDefaults.standard.set(String(id.dropFirst(9)), forKey: "followUpBehavior")
            reload()
        case let id where id.hasPrefix("chatMission:"):
            ChatMission.lead = id == "chatMission:lead"
            reload()
        case let id where id.hasPrefix("appearance:"):
            let style = Int(id.dropFirst(11)) ?? 0
            UserDefaults.standard.set(style, forKey: "appearance")
            view.window?.overrideUserInterfaceStyle = UIUserInterfaceStyle(rawValue: style) ?? .unspecified
            reload()
        case "wallpaper:choose":
            WallpaperPicker.present(from: self)
        case "wallpaper:host":
            let sheet = UIAlertController(title: "Sync with Mac", message: nil, preferredStyle: .actionSheet)
            let current = WallpaperSync.host(app)?.id
            for host in app.client?.wallpaperHosts() ?? [] {
                let action = UIAlertAction(title: host.name, style: .default) { [weak self] _ in
                    guard let self else { return }
                    WallpaperStore.syncDeviceId = host.id
                    WallpaperSync.refresh(self.app)
                    self.reload()
                }
                action.setValue(host.id == current, forKey: "checked")
                sheet.addAction(action)
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = collectionView.cellForItem(at: path)
            present(sheet, animated: true)
        case "wallpaper:effect":
            let sheet = UIAlertController(title: "Wallpaper Effect", message: nil, preferredStyle: .actionSheet)
            for e in WallpaperStore.allEffects {
                let action = UIAlertAction(title: WallpaperStore.label(e), style: .default) { _ in WallpaperStore.effect = e }
                action.setValue(e == WallpaperStore.effect, forKey: "checked")
                sheet.addAction(action)
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = collectionView.cellForItem(at: path)
            present(sheet, animated: true)
        case "math:font":
            let sheet = UIAlertController(title: "Math Font", message: nil, preferredStyle: .actionSheet)
            for f in MathSettings.allFaces {
                let action = UIAlertAction(title: MathSettings.label(f), style: .default) { [weak self] _ in
                    MathSettings.face = f
                    self?.reload()
                }
                action.setValue(f == MathSettings.face, forKey: "checked")
                sheet.addAction(action)
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = collectionView.cellForItem(at: path)
            present(sheet, animated: true)
        case "math:size":
            let sheet = UIAlertController(title: "Math Size", message: nil, preferredStyle: .actionSheet)
            for size in MathSettings.allSizes {
                let action = UIAlertAction(title: "\(size)%", style: .default) { [weak self] _ in
                    MathSettings.size = size
                    self?.reload()
                }
                action.setValue(size == MathSettings.size, forKey: "checked")
                sheet.addAction(action)
            }
            sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            sheet.popoverPresentationController?.sourceView = collectionView.cellForItem(at: path)
            present(sheet, animated: true)
        case "wallpaper:remove":
            WallpaperStore.remove()
        case "whatsnew":
            WhatsNewViewController.present(from: self)
        case "docs":
            navigationController?.pushViewController(DocsViewController(), animated: true)
        case "archived":
            navigationController?.pushViewController(FolderViewController(app: app, folder: FolderRowVM(id: "archived", name: "Archived", count: 0, symbol: "archivebox")), animated: true)
        case "signout":
            let alert = UIAlertController(title: "Sign out?", message: "Messages still waiting to send and the New session draft are deleted. Text left in a session's composer stays on this device.", preferredStyle: .actionSheet)
            alert.addAction(UIAlertAction(title: "Sign Out", style: .destructive) { [weak self] _ in self?.app.signOut() })
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            alert.popoverPresentationController?.sourceView = collectionView.cellForItem(at: path)
            present(alert, animated: true)
        default:
            break
        }
    }
}
