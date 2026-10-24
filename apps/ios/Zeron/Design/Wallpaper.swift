import CoreImage
import ImageIO
import PhotosUI
import UIKit
import UniformTypeIdentifiers

/// The chat wallpaper (desktop "new thread background"): one image plus an
/// effect, stored locally. Effects and the contrast guard run in the Rust core
/// (`wallpaper.rs`) off the main thread; renders are cached per appearance.
enum WallpaperStore {
    static let didChange = Notification.Name("ZeronWallpaperChanged")

    private static var directory: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("wallpaper", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private static var localImageURL: URL { directory.appendingPathComponent("wallpaper.jpg") }
    fileprivate static var syncedImageURL: URL { directory.appendingPathComponent("synced.jpg") }
    private static var imageURL: URL { following != nil ? syncedImageURL : localImageURL }

    static var isSet: Bool { FileManager.default.fileExists(atPath: imageURL.path) }

    struct ChatBackground: Codable, Equatable {
        enum Placement: String, Codable, CaseIterable {
            case hero, full

            var label: String { self == .hero ? "Hero" : "Full" }
            var detail: String { self == .hero ? "Same top crop as new threads, fading out below." : "Covers the whole conversation column." }
        }

        static let blurMax: CGFloat = 48
        static let blurStep: CGFloat = 4

        var enabled = false
        var placement = Placement.hero
        var opacity: CGFloat = 0.2
        var blur: CGFloat = 24
        var scrim = true

        var visible: Bool { enabled && opacity > 0 }

        func clamped() -> ChatBackground {
            var c = self
            c.opacity = opacity.isFinite ? min(1, max(0, opacity)) : 0.2
            let blur = blur.isFinite ? min(Self.blurMax, max(0, blur)) : 24
            c.blur = (blur / Self.blurStep).rounded() * Self.blurStep
            return c
        }
    }

    struct Synced: Codable, Equatable {
        var deviceId: String
        var name: String?
        var hash: String?
        var effect: String
        var chat: ChatBackground
    }

    static var syncWithMac: Bool {
        get { UserDefaults.standard.bool(forKey: "wallpaperSync") }
        set {
            UserDefaults.standard.set(newValue, forKey: "wallpaperSync")
            changed()
        }
    }

    static var syncDeviceId: String? {
        get { UserDefaults.standard.string(forKey: "wallpaperSyncDevice") }
        set { UserDefaults.standard.set(newValue, forKey: "wallpaperSyncDevice") }
    }

    static var synced: Synced? {
        get { UserDefaults.standard.data(forKey: "wallpaperSynced").flatMap { try? JSONDecoder().decode(Synced.self, from: $0) } }
        set { UserDefaults.standard.set(newValue.flatMap { try? JSONEncoder().encode($0) }, forKey: "wallpaperSynced") }
    }

    static var following: Synced? { syncWithMac ? synced : nil }

    static var name: String? { following.map(\.name) ?? UserDefaults.standard.string(forKey: "wallpaperName") }

    static var effect: WallpaperEffect {
        get { effectNamed(following?.effect ?? UserDefaults.standard.string(forKey: "wallpaperEffect")) }
        set {
            UserDefaults.standard.set(key(newValue), forKey: "wallpaperEffect")
            changed()
        }
    }

    static var chatBackground: ChatBackground {
        get {
            if let following { return following.chat }
            let d = UserDefaults.standard
            var c = ChatBackground()
            c.enabled = d.bool(forKey: "chatBackground")
            if let p = d.string(forKey: "chatBackgroundPlacement").flatMap(ChatBackground.Placement.init) { c.placement = p }
            if d.object(forKey: "chatBackgroundOpacity") != nil { c.opacity = CGFloat(d.double(forKey: "chatBackgroundOpacity")) }
            if d.object(forKey: "chatBackgroundBlur") != nil { c.blur = CGFloat(d.double(forKey: "chatBackgroundBlur")) }
            if d.object(forKey: "chatBackgroundScrim") != nil { c.scrim = d.bool(forKey: "chatBackgroundScrim") }
            return c.clamped()
        }
        set {
            let c = newValue.clamped()
            let d = UserDefaults.standard
            d.set(c.enabled, forKey: "chatBackground")
            d.set(c.placement.rawValue, forKey: "chatBackgroundPlacement")
            d.set(Double(c.opacity), forKey: "chatBackgroundOpacity")
            d.set(Double(c.blur), forKey: "chatBackgroundBlur")
            d.set(c.scrim, forKey: "chatBackgroundScrim")
            NotificationCenter.default.post(name: didChange, object: nil)
        }
    }

    static func effectNamed(_ key: String?) -> WallpaperEffect {
        allEffects.first { Self.key($0) == key } ?? .none
    }

    fileprivate static func store(_ next: Synced, image: Data?) {
        if let image {
            try? image.write(to: syncedImageURL, options: .atomic)
        } else if next.hash == nil {
            try? FileManager.default.removeItem(at: syncedImageURL)
        }
        guard image != nil || next != synced else { return }
        synced = next
        changed()
    }

    static func key(_ e: WallpaperEffect) -> String {
        switch e {
        case .none: "none"
        case .dither: "dither"
        case .ascii: "ascii"
        case .halftone: "halftone"
        case .scanlines: "scanlines"
        }
    }

    static let allEffects: [WallpaperEffect] = [.none, .dither, .ascii, .halftone, .scanlines]

    /// Desktop labels / descriptions.
    static func label(_ e: WallpaperEffect) -> String {
        switch e {
        case .none: "None"
        case .dither: "Dither"
        case .ascii: "ASCII"
        case .halftone: "Halftone"
        case .scanlines: "Scanlines"
        }
    }

    static func detail(_ e: WallpaperEffect) -> String {
        switch e {
        case .none: "Shows the original artwork."
        case .dither: "Rebuilds the artwork with a dithered color palette."
        case .ascii: "Recreates the artwork with colored characters."
        case .halftone: "Recreates the artwork with colored print dots."
        case .scanlines: "Adds a pronounced horizontal display-line texture."
        }
    }

    /// Store a picked image (long side ≤ 1600 px) and notify.
    static func set(_ image: UIImage, name: String) {
        let scale = min(1, 1600 / max(image.size.width * image.scale, image.size.height * image.scale))
        let size = CGSize(width: (image.size.width * image.scale * scale).rounded(), height: (image.size.height * image.scale * scale).rounded())
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let resized = UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
        try? resized.jpegData(compressionQuality: 0.9)?.write(to: localImageURL, options: .atomic)
        UserDefaults.standard.set(name, forKey: "wallpaperName")
        changed()
    }

    /// Store a picked photo straight from its file, downsampled while it's
    /// decoded (a 48MP photo never becomes a full-size bitmap), off the main
    /// thread; notifies on main.
    static func set(fileURL: URL, name: String) {
        let options = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithURL(fileURL as CFURL, options),
              let thumb = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                  kCGImageSourceCreateThumbnailFromImageAlways: true,
                  kCGImageSourceCreateThumbnailWithTransform: true,
                  kCGImageSourceThumbnailMaxPixelSize: 1600,
              ] as CFDictionary),
              let data = UIImage(cgImage: thumb).jpegData(compressionQuality: 0.9)
        else { return }
        try? data.write(to: localImageURL, options: .atomic)
        DispatchQueue.main.async {
            UserDefaults.standard.set(name, forKey: "wallpaperName")
            changed()
        }
    }

    static func remove() {
        try? FileManager.default.removeItem(at: localImageURL)
        UserDefaults.standard.removeObject(forKey: "wallpaperName")
        changed()
    }

    private static func changed() {
        lock.lock()
        cache.removeAll()
        generation += 1
        lock.unlock()
        NotificationCenter.default.post(name: didChange, object: nil)
    }

    /// A rendered wallpaper and the highest opacity that keeps text readable.
    struct Render {
        let image: UIImage
        let opacity: CGFloat
    }

    /// `render` blurred heavily (off the main thread): its colours without
    /// its detail. Delivered on the main queue.
    static func soften(_ render: Render, done: @escaping (Render) -> Void) {
        queue.async {
            let soft = blurred(render.image).map { Render(image: $0, opacity: render.opacity) } ?? render
            DispatchQueue.main.async { done(soft) }
        }
    }

    private static func blurred(_ image: UIImage) -> UIImage? {
        // Small first: a heavy blur needs no detail, and it's cheap.
        let width: CGFloat = 240
        let size = CGSize(width: width, height: (width * image.size.height / max(1, image.size.width)).rounded())
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let small = UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
        guard let input = CIImage(image: small) else { return nil }
        let output = input.clampedToExtent()
            .applyingFilter("CIGaussianBlur", parameters: [kCIInputRadiusKey: 14])
            .applyingFilter("CIColorControls", parameters: [kCIInputSaturationKey: 1.15])
            .cropped(to: input.extent)
        guard let cg = CIContext().createCGImage(output, from: input.extent) else { return nil }
        return UIImage(cgImage: cg)
    }

    private static let lock = NSLock()
    private static var cache: [String: Render] = [:]
    private static var generation = 0
    private static let queue = DispatchQueue(label: "sh.zeron.wallpaper", qos: .userInitiated)

    /// Render for an appearance; `completion` runs on the main thread (nil
    /// when no wallpaper is set). Cached until the image or effect changes.
    /// `blur` is the desktop's chat background blur, px at 1400 px wide.
    static func render(dark: Bool, blur: CGFloat = 0, completion: @escaping (Render?) -> Void) {
        guard isSet else { return completion(nil) }
        let effect = self.effect
        let key = "\(Self.key(effect))-\(dark)-\(Int(blur))"
        lock.lock()
        let hit = cache[key]
        let gen = generation
        lock.unlock()
        if let hit { return completion(hit) }
        let traits = UITraitCollection(userInterfaceStyle: dark ? .dark : .light)
        let text = rgb(Palette.text.resolvedColor(with: traits))
        let secondary = rgb(Palette.secondary.resolvedColor(with: traits))
        let background = rgb(Palette.background.resolvedColor(with: traits))
        let url = imageURL
        queue.async {
            guard let source = UIImage(contentsOfFile: url.path)?.cgImage,
                  let (pixels, w, h) = rgba(source, maxSide: 1200)
            else { return DispatchQueue.main.async { completion(nil) } }
            let out = wallpaperRender(rgba: pixels, width: UInt32(w), height: UInt32(h), effect: effect, light: !dark)
            // Text sits over the top of the artwork: primary text keeps 4.5:1,
            // secondary 3:1 against the artwork's worst pixels there.
            let region: Float = 0.6
            let primary = wallpaperSafeOpacity(rgba: out, width: UInt32(w), height: UInt32(h), textRgb: text, backgroundRgb: background, region: region, minContrast: 4.5, maxOpacity: 1)
            let muted = wallpaperSafeOpacity(rgba: out, width: UInt32(w), height: UInt32(h), textRgb: secondary, backgroundRgb: background, region: region, minContrast: 3, maxOpacity: 1)
            guard let image = cgImage(out, w, h).flatMap({ blur > 0 ? blurred($0, sigma: blur * CGFloat(w) / 1400) : $0 }) else { return DispatchQueue.main.async { completion(nil) } }
            let render = Render(image: UIImage(cgImage: image), opacity: CGFloat(min(primary, muted)))
            lock.lock()
            if gen == generation { cache[key] = render }
            lock.unlock()
            DispatchQueue.main.async { completion(render) }
        }
    }

    private static func blurred(_ image: CGImage, sigma: CGFloat) -> CGImage? {
        let input = CIImage(cgImage: image)
        let output = input.clampedToExtent()
            .applyingFilter("CIGaussianBlur", parameters: [kCIInputRadiusKey: sigma])
            .cropped(to: input.extent)
        return CIContext().createCGImage(output, from: input.extent)
    }

    private static func rgb(_ c: UIColor) -> UInt32 {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        c.getRed(&r, green: &g, blue: &b, alpha: &a)
        return UInt32(r * 255) << 16 | UInt32(g * 255) << 8 | UInt32(b * 255)
    }

    private static func rgba(_ image: CGImage, maxSide: CGFloat) -> (Data, Int, Int)? {
        let scale = min(1, maxSide / CGFloat(max(image.width, image.height)))
        let w = max(1, Int(CGFloat(image.width) * scale)), h = max(1, Int(CGFloat(image.height) * scale))
        var data = Data(count: w * h * 4)
        let ok = data.withUnsafeMutableBytes { buf -> Bool in
            guard let ctx = CGContext(data: buf.baseAddress, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            ctx.interpolationQuality = .high
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
            return true
        }
        return ok ? (data, w, h) : nil
    }

    private static func cgImage(_ data: Data, _ w: Int, _ h: Int) -> CGImage? {
        guard let provider = CGDataProvider(data: data as CFData) else { return nil }
        return CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue), provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }
}

enum WallpaperSync {
    private static var inFlight = false

    static func host(_ app: AppModel) -> DeviceView? {
        let hosts = app.client?.wallpaperHosts() ?? []
        return hosts.first { $0.id == WallpaperStore.syncDeviceId } ?? hosts.first
    }

    static func refresh(_ app: AppModel) {
        guard WallpaperStore.syncWithMac, !inFlight, let client = app.client, let host = host(app) else { return }
        let previous = WallpaperStore.synced
        let have = previous?.deviceId == host.id && FileManager.default.fileExists(atPath: WallpaperStore.syncedImageURL.path)
        let known = have ? previous?.hash : nil
        inFlight = true
        Task { @MainActor in
            defer { inFlight = false }
            guard let fetched = try? await client.chatWallpaper(deviceId: host.id, knownHash: known) else { return }
            let chat = WallpaperStore.ChatBackground(
                enabled: fetched.enabled,
                placement: WallpaperStore.ChatBackground.Placement(rawValue: fetched.placement) ?? .hero,
                opacity: CGFloat(fetched.opacity),
                blur: CGFloat(fetched.blur),
                scrim: fetched.scrim
            ).clamped()
            let next = WallpaperStore.Synced(deviceId: host.id, name: fetched.name, hash: fetched.imageHash, effect: fetched.effect, chat: chat)
            WallpaperStore.store(next, image: fetched.image)
        }
    }
}

/// The wallpaper hero: aspect-filled artwork at the top of a page that fades
/// out downward (alpha, never a tinted overlay — the page shows through), at
/// the opacity the contrast guard allows. An optional cutout (the new-chat
/// composer) softens the artwork behind it, like desktop.
final class WallpaperView: UIView {
    private let imageView = UIImageView()
    private let maskLayer = CALayer()
    private let scrim = UIView()
    private let scrimTop = CAGradientLayer()
    private let scrimBottom = CAGradientLayer()
    private var render: WallpaperStore.Render?
    private var chat = WallpaperStore.ChatBackground()
    private var maskKey: (CGSize, CGRect?)?
    private var observer: NSObjectProtocol?
    /// Extra fade applied by the host (e.g. scrolling the list away).
    var scrollFade: CGFloat = 1 { didSet { applyAlpha() } }
    /// A rect (in this view's coordinates) to soften — the composer.
    var cutout: CGRect? { didSet { if cutout != oldValue { setNeedsLayout() } } }
    /// Blurred to a wash of its colours, for pages whose text runs over it.
    var softened = false { didSet { if softened != oldValue { reload() } } }
    /// Fills its host and draws the desktop's chat background inside.
    var behindChat = false

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        clipsToBounds = true
        imageView.contentMode = .scaleAspectFill
        imageView.clipsToBounds = true
        imageView.layer.mask = maskLayer
        addSubview(imageView)
        scrim.layer.addSublayer(scrimTop)
        scrim.layer.addSublayer(scrimBottom)
        scrim.isHidden = true
        addSubview(scrim)
        alpha = 0
        observer = NotificationCenter.default.addObserver(forName: WallpaperStore.didChange, object: nil, queue: .main) { [weak self] _ in self?.reload() }
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (self: WallpaperView, _) in self.reload() }
        accessibilityIdentifier = "wallpaper"
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit { observer.map(NotificationCenter.default.removeObserver) }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil, render == nil { reload() }
    }

    func reload() {
        let dark = traitCollection.userInterfaceStyle == .dark
        let chat = WallpaperStore.chatBackground
        if behindChat, !chat.visible {
            self.chat = chat
            return show(nil)
        }
        WallpaperStore.render(dark: dark, blur: behindChat ? chat.blur : 0) { [weak self] render in
            guard let self else { return }
            self.chat = chat
            if self.softened, let render {
                WallpaperStore.soften(render) { [weak self] soft in
                    guard let self, self.softened else { return }
                    self.show(soft)
                }
            } else {
                self.show(render)
            }
        }
    }

    private func show(_ render: WallpaperStore.Render?) {
        let first = self.render == nil
        self.render = render
        imageView.image = render?.image
        imageView.alpha = behindChat ? chat.opacity : render?.opacity ?? 0
        scrim.isHidden = !(behindChat && chat.scrim && render != nil)
        isHidden = render == nil
        setNeedsLayout()
        // Artwork arriving late fades in (desktop: 120 ms).
        if first, render != nil, window != nil, !UIAccessibility.isReduceMotionEnabled {
            alpha = 0
            UIView.animate(withDuration: 0.12) { self.applyAlpha() }
        } else {
            applyAlpha()
        }
    }

    private func applyAlpha() {
        alpha = render == nil ? 0 : max(0, min(1, scrollFade))
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let full = behindChat && chat.placement == .full
        let frame = behindChat && !full ? CGRect(x: 0, y: 0, width: bounds.width, height: min(bounds.height * 0.72, 760)) : bounds
        imageView.frame = frame
        imageView.layer.mask = full ? nil : maskLayer
        if !full, maskKey?.0 != frame.size || maskKey?.1 != cutout {
            maskKey = (frame.size, cutout)
            maskLayer.frame = CGRect(origin: .zero, size: frame.size)
            maskLayer.contents = Self.mask(size: frame.size, cutout: cutout)
        }
        scrim.frame = frame
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let bg = Palette.background.resolvedColor(with: traitCollection)
        let split = frame.height * 0.45
        scrimTop.frame = CGRect(x: 0, y: 0, width: frame.width, height: split)
        scrimTop.colors = [bg.withAlphaComponent(0.10).cgColor, bg.withAlphaComponent(0.40).cgColor]
        scrimBottom.frame = CGRect(x: 0, y: split, width: frame.width, height: frame.height - split)
        scrimBottom.colors = [bg.withAlphaComponent(0.40).cgColor, bg.withAlphaComponent(full ? 0.75 : 0).cgColor]
        CATransaction.commit()
    }

    /// Alpha mask: full at the top fading to clear at the bottom (eased, over
    /// the full height as on desktop), halved inside a feathered cutout.
    private static func mask(size: CGSize, cutout: CGRect?) -> CGImage? {
        guard size.width > 0, size.height > 0 else { return nil }
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = false
        return UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            let c = ctx.cgContext
            let steps = 24
            var colors: [CGColor] = []
            var locations: [CGFloat] = []
            for i in 0...steps {
                let t = CGFloat(i) / CGFloat(steps)
                // Smoothstep-ish ease so the fade has no visible band.
                let a = 1 - (t * t * (3 - 2 * t))
                colors.append(UIColor(white: 0, alpha: a).cgColor)
                locations.append(t)
            }
            if let g = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors as CFArray, locations: locations) {
                c.drawLinearGradient(g, start: .zero, end: CGPoint(x: 0, y: size.height), options: [])
            }
            if let cut = cutout, cut.intersects(CGRect(origin: .zero, size: size)) {
                let feather = min(280, max(120, size.height * 0.52)) / 3
                c.setBlendMode(.destinationOut)
                c.setShadow(offset: .zero, blur: feather, color: UIColor(white: 0, alpha: 0.5).cgColor)
                UIColor(white: 0, alpha: 0.5).setFill()
                UIBezierPath(roundedRect: cut.insetBy(dx: -8, dy: -8), cornerRadius: 26).fill()
            }
        }.cgImage
    }
}

/// Picks a wallpaper image from Photos.
final class WallpaperPicker: NSObject, PHPickerViewControllerDelegate {
    private var retainSelf: WallpaperPicker?

    static func present(from host: UIViewController) {
        var config = PHPickerConfiguration()
        config.filter = .images
        config.selectionLimit = 1
        let picker = PHPickerViewController(configuration: config)
        let delegate = WallpaperPicker()
        delegate.retainSelf = delegate
        picker.delegate = delegate
        host.present(picker, animated: true)
    }

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        guard let provider = results.first?.itemProvider, provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) else {
            retainSelf = nil
            return
        }
        let name = provider.suggestedName ?? "Photo"
        // The file, not a decoded UIImage: it's downsampled as it's read.
        provider.loadFileRepresentation(forTypeIdentifier: UTType.image.identifier) { [weak self] url, _ in
            if let url { WallpaperStore.set(fileURL: url, name: name) }
            self?.retainSelf = nil
        }
    }
}
