import UIKit

final class WhatsNewViewController: UIViewController {
    private let entries: [ChangelogEntry]
    private let full: Bool

    static func present(from host: UIViewController) {
        guard let entry = whatsNew() else { return }
        let nav = UINavigationController(rootViewController: WhatsNewViewController(entries: [entry], full: false))
        nav.sheetPresentationController?.detents = [.medium(), .large()]
        nav.sheetPresentationController?.prefersGrabberVisible = true
        host.present(nav, animated: true)
    }

    init(entries: [ChangelogEntry], full: Bool) {
        self.entries = entries
        self.full = full
        super.init(nibName: nil, bundle: nil)
        title = full ? "Changelog" : "What's new?"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        if !full {
            navigationItem.rightBarButtonItem = UIBarButtonItem(title: "Got it", primaryAction: UIAction { [weak self] _ in
                self?.dismiss(animated: true)
            })
        }
        let scroll = UIScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        let stack = UIStackView()
        stack.axis = .vertical
        stack.spacing = 18
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 16),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -24),
            stack.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor, constant: -20),
        ])
        for entry in entries {
            stack.addArrangedSubview(label("v\(entry.version) · \(entry.date)", font: Fonts.ui(.sansMedium, 13), color: Palette.secondary))
            for feature in entry.features {
                let block = UIStackView(arrangedSubviews: [
                    label(feature.title, font: Fonts.ui(.sansMedium, 17), color: Palette.text),
                    label(feature.description, font: Fonts.ui(.sans, 15), color: Palette.secondary),
                ])
                block.axis = .vertical
                block.spacing = 4
                stack.addArrangedSubview(block)
            }
        }
        if !full {
            var config = UIButton.Configuration.plain()
            config.title = "View changelog →"
            config.contentInsets = .zero
            let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in
                self?.navigationController?.pushViewController(WhatsNewViewController(entries: changelog(), full: true), animated: true)
            })
            button.tintColor = Palette.accent
            button.contentHorizontalAlignment = .leading
            button.accessibilityIdentifier = "whatsnew-changelog"
            stack.addArrangedSubview(button)
        }
    }

    private func label(_ text: String, font: UIFont, color: UIColor) -> UILabel {
        let label = UILabel()
        label.text = text
        label.font = font
        label.textColor = color
        label.numberOfLines = 0
        return label
    }
}

final class DocsViewController: UIViewController, UICollectionViewDelegate {
    private let sections = docsSections()
    private var collectionView: UICollectionView!

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Docs"
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        config.footerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.dataSource = self
        collectionView.register(UICollectionViewListCell.self, forCellWithReuseIdentifier: "cell")
        collectionView.register(UICollectionViewListCell.self, forSupplementaryViewOfKind: UICollectionView.elementKindSectionHeader, withReuseIdentifier: "header")
        collectionView.register(UICollectionViewListCell.self, forSupplementaryViewOfKind: UICollectionView.elementKindSectionFooter, withReuseIdentifier: "footer")
        view.addSubview(collectionView)
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt path: IndexPath) {
        collectionView.deselectItem(at: path, animated: true)
        let article = sections[path.section].articles[path.item]
        navigationController?.pushViewController(DocArticleViewController(id: article.id, title: article.title), animated: true)
    }
}

extension DocsViewController: UICollectionViewDataSource {
    func numberOfSections(in collectionView: UICollectionView) -> Int { sections.count }

    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int {
        sections[section].articles.count
    }

    func collectionView(_ collectionView: UICollectionView, cellForItemAt path: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "cell", for: path) as! UICollectionViewListCell
        let article = sections[path.section].articles[path.item]
        var c = UIListContentConfiguration.subtitleCell()
        c.text = article.title
        c.textProperties.font = Fonts.ui(.sansMedium, 16)
        c.textProperties.color = Palette.text
        c.secondaryText = article.blurb
        c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
        c.secondaryTextProperties.color = Palette.secondary
        cell.contentConfiguration = c
        var bg = UIBackgroundConfiguration.listGroupedCell()
        bg.backgroundColor = Palette.elevated
        cell.backgroundConfiguration = bg
        cell.accessories = [.disclosureIndicator()]
        cell.accessibilityIdentifier = "doc-\(article.id)"
        return cell
    }

    func collectionView(_ collectionView: UICollectionView, viewForSupplementaryElementOfKind kind: String, at path: IndexPath) -> UICollectionReusableView {
        let header = kind == UICollectionView.elementKindSectionHeader
        let view = collectionView.dequeueReusableSupplementaryView(ofKind: kind, withReuseIdentifier: header ? "header" : "footer", for: path) as! UICollectionViewListCell
        var c = header ? UIListContentConfiguration.groupedHeader() : UIListContentConfiguration.groupedFooter()
        c.text = header ? sections[path.section].title : (path.section == sections.count - 1 ? "Links to settings point at the Mac app; links to other articles open here." : nil)
        view.contentConfiguration = c
        return view
    }
}

final class DocArticleViewController: UIViewController {
    private let id: String
    private lazy var relay = FrameRelay { [weak self] in self?.applyFrame() }
    private lazy var engine = TranscriptView(text: TextEngine.shared, listener: relay)
    private lazy var list = TranscriptListView(engine: engine)
    private var placed = false

    init(id: String, title: String) {
        self.id = id
        super.init(nibName: nil, bundle: nil)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        list.frame = view.bounds
        list.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        list.accessibilityIdentifier = "doc-article"
        list.onLink = { [weak self] url in self?.open(url) ?? false }
        view.addSubview(list)
        setContentScrollView(list, for: .top)
        MathSettings.apply(to: engine)
        engine.setDebugEntries(entries: [DebugEntry(id: id, user: false, text: docsBody(id: id) ?? "", streaming: false)], working: false)
    }

    private func applyFrame() {
        list.apply(engine.frame())
        if !placed, engine.frame().rowCount() > 0 {
            placed = true
            list.setContentOffset(CGPoint(x: 0, y: -list.adjustedContentInset.top), animated: false)
        }
    }

    private func open(_ url: URL) -> Bool {
        guard url.scheme == "zeron" else { return false }
        if let id = docsLink(url: url.absoluteString), let article = docsSections().flatMap(\.articles).first(where: { $0.id == id }) {
            navigationController?.pushViewController(DocArticleViewController(id: id, title: article.title), animated: true)
        } else {
            Toast.show("Open this in Zeron on the Mac", in: view.window)
        }
        return true
    }
}
