import UIKit

final class HtmlSheetViewController: UIViewController {
    private let pageTitle: String
    private let page: HtmlPageView
    private let source = UITextView()
    private let html: () async -> String?
    private var loaded: String?

    init(title: String, html: @escaping () async -> String?, openURL: @escaping (URL) -> Void) {
        pageTitle = title
        self.html = html
        page = HtmlPageView(source: html)
        super.init(nibName: nil, bundle: nil)
        page.scrolls = true
        page.openURL = { [weak self] url in self?.dismiss(animated: true) { openURL(url) } }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.title = pageTitle
        let segments = UISegmentedControl(items: ["Live", "Source"])
        segments.selectedSegmentIndex = 0
        segments.accessibilityIdentifier = "html-sheet-view"
        segments.addAction(UIAction { [weak self, weak segments] _ in
            self?.show(source: segments?.selectedSegmentIndex == 1)
        }, for: .valueChanged)
        let close = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in self?.dismiss(animated: true) })
        close.accessibilityIdentifier = "html-sheet-close"
        let save = UIBarButtonItem(title: "Save .html", primaryAction: UIAction { [weak self] _ in self?.save() })
        save.accessibilityIdentifier = "html-sheet-save"
        navigationItem.leftBarButtonItem = close
        navigationItem.rightBarButtonItem = save
        toolbarItems = [.flexibleSpace(), UIBarButtonItem(customView: segments), .flexibleSpace()]
        navigationController?.isToolbarHidden = false

        source.isEditable = false
        source.backgroundColor = .clear
        source.textColor = Palette.text
        source.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: .monospacedSystemFont(ofSize: 12, weight: .regular))
        source.textContainerInset = UIEdgeInsets(top: 16, left: 12, bottom: 16, right: 12)
        source.accessibilityIdentifier = "html-sheet-source"
        source.isHidden = true
        for v in [page, source] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
            NSLayoutConstraint.activate([
                v.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
                v.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
                v.leadingAnchor.constraint(equalTo: view.leadingAnchor),
                v.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            ])
        }
    }

    private func text() async -> String? {
        if let loaded { return loaded }
        loaded = await html()
        return loaded
    }

    private func show(source shown: Bool) {
        page.isHidden = shown
        source.isHidden = !shown
        guard shown, source.text.isEmpty else { return }
        Task { @MainActor [weak self] in
            guard let html = await self?.text() else { return }
            self?.source.text = htmlRenderSource(html: html)
        }
    }

    private func save() {
        Task { @MainActor [weak self] in
            guard let self, let html = await self.text() else { return }
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(htmlRenderFileName(title: self.pageTitle))
            guard (try? html.write(to: url, atomically: true, encoding: .utf8)) != nil else { return }
            self.present(UIDocumentPickerViewController(forExporting: [url], asCopy: true), animated: true)
        }
    }
}
