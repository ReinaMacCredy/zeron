import UIKit
import WebKit

final class HtmlPageView: UIView, WKNavigationDelegate, WKUIDelegate {
    var onHeight: ((CGFloat) -> Void)?
    var openURL: ((URL) -> Void)?
    var scrolls = false {
        didSet { web.scrollView.isScrollEnabled = scrolls }
    }
    private let web: WKWebView
    private let source: () async -> String?
    private var html: String?
    private var loadedOnce = false
    private var crashes = 0
    private var rendered: (UIUserInterfaceStyle, Float)?

    init(source: @escaping () async -> String?) {
        self.source = source
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.preferences.javaScriptCanOpenWindowsAutomatically = false
        config.allowsInlineMediaPlayback = true
        config.dataDetectorTypes = []
        web = WKWebView(frame: .zero, configuration: config)
        super.init(frame: .zero)
        config.userContentController.add(HeightRelay(self), name: "zeron")
        web.isOpaque = false
        web.backgroundColor = .clear
        web.scrollView.backgroundColor = .clear
        web.scrollView.isScrollEnabled = false
        web.scrollView.contentInsetAdjustmentBehavior = .never
        web.scrollView.showsVerticalScrollIndicator = false
        web.allowsLinkPreview = false
        web.navigationDelegate = self
        web.uiDelegate = self
        web.frame = bounds
        web.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        web.accessibilityIdentifier = "html-render-page"
        addSubview(web)
        registerForTraitChanges([UITraitUserInterfaceStyle.self, UITraitPreferredContentSizeCategory.self]) { (self: HtmlPageView, _) in
            self.render()
        }
        Task { @MainActor [weak self] in
            guard let html = await source() else { return }
            self?.html = html
            self?.render()
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit {
        web.configuration.userContentController.removeScriptMessageHandler(forName: "zeron")
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        render()
    }

    private func render() {
        guard let html, window != nil else { return }
        let key = (traitCollection.userInterfaceStyle, Float(UIFontMetrics(forTextStyle: .body).scaledValue(for: 17) / 17))
        if let rendered, rendered == key { return }
        rendered = key
        loadedOnce = false
        web.loadHTMLString(htmlRenderDocument(html: html, dark: key.0 == .dark, textScale: key.1), baseURL: nil)
    }

    fileprivate func report(_ body: Any) {
        guard let text = body as? String, let data = text.data(using: .utf8),
              let message = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let height = (message["height"] as? NSNumber)?.doubleValue
        else { return }
        onHeight?(CGFloat(height))
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        let url = action.request.url
        let address = url?.absoluteString ?? ""
        guard let frame = action.targetFrame else { return decisionHandler(.cancel) }
        if !frame.isMainFrame { return decisionHandler(url?.isFileURL == true ? .cancel : .allow) }
        if !loadedOnce {
            loadedOnce = true
            return decisionHandler(address == "about:blank" ? .allow : .cancel)
        }
        if address.hasPrefix("about:blank#") { return decisionHandler(.allow) }
        if action.navigationType == .linkActivated, let url, Self.leavesForBrowser(url) { openURL?(url) }
        decisionHandler(.cancel)
    }

    func webView(_ webView: WKWebView, decidePolicyFor response: WKNavigationResponse, decisionHandler: @escaping @MainActor (WKNavigationResponsePolicy) -> Void) {
        decisionHandler(response.canShowMIMEType ? .allow : .cancel)
    }

    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if action.navigationType == .linkActivated, let url = action.request.url, Self.leavesForBrowser(url) { openURL?(url) }
        return nil
    }

    func webView(_ webView: WKWebView, requestMediaCapturePermissionFor origin: WKSecurityOrigin, initiatedByFrame frame: WKFrameInfo, type: WKMediaCaptureType, decisionHandler: @escaping @MainActor (WKPermissionDecision) -> Void) {
        decisionHandler(.deny)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        crashes += 1
        guard crashes <= 2 else { return }
        rendered = nil
        render()
    }

    private static func leavesForBrowser(_ url: URL) -> Bool {
        (url.scheme == "http" || url.scheme == "https") && url.host() != nil && url.user == nil && url.password == nil
    }
}

private final class HeightRelay: NSObject, WKScriptMessageHandler {
    weak var page: HtmlPageView?

    init(_ page: HtmlPageView) { self.page = page }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard message.frameInfo.isMainFrame else { return }
        page?.report(message.body)
    }
}
