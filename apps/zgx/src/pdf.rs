use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::UNIX_EPOCH,
};

use core_foundation::{base::TCFType, url::CFURL};
use core_graphics::{
    base::{kCGBitmapByteOrder32Big, kCGImageAlphaPremultipliedLast, CGFloat},
    color_space::CGColorSpace,
    context::{CGContext, CGInterpolationQuality},
    geometry::{CGAffineTransform, CGPoint, CGRect, CGSize},
};
use foreign_types::ForeignType;
use sha2::{Digest, Sha256};
use zgx_script::{ScriptHost, Value};

const STYLE_VERSION: u32 = 2;
/// Past these a page is drawn less dense, bounding its texture and memory.
const MAX_SIDE: f64 = 8192.;
const MAX_PIXELS: f64 = 32. * 1024. * 1024.;
const CACHE_BYTES: u64 = 512 * 1024 * 1024;
const KEEP_OPEN: usize = 4;
const CROP_BOX: i32 = 1;

type DocRef = *mut c_void;
type PageRef = *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPDFDocumentCreateWithURL(url: core_foundation::url::CFURLRef) -> DocRef;
    fn CGPDFDocumentRelease(document: DocRef);
    fn CGPDFDocumentGetNumberOfPages(document: DocRef) -> usize;
    fn CGPDFDocumentIsEncrypted(document: DocRef) -> bool;
    fn CGPDFDocumentIsUnlocked(document: DocRef) -> bool;
    fn CGPDFDocumentGetPage(document: DocRef, number: usize) -> PageRef;
    fn CGPDFPageGetBoxRect(page: PageRef, which: i32) -> CGRect;
    fn CGPDFPageGetRotationAngle(page: PageRef) -> i32;
    fn CGContextDrawPDFPage(context: *mut core_graphics::sys::CGContext, page: PageRef);
}

pub fn install(host: &ScriptHost, display_scale: impl Fn() -> f64 + 'static) {
    host.async_action("pdf.open", |arg| async move {
        let path = path_of(&arg)?;
        let (reply, mut answer) = zgx::task::latest();
        submit(Job::Open { path, reply });
        let pages = answer.recv().await.ok_or(DAMAGED)??;
        let list: Vec<Value> = pages
            .into_iter()
            .map(|(w, h)| Value::record([("w", Value::from(w)), ("h", Value::from(h))]))
            .collect();
        Ok(Value::record([("pages", Value::List(list.into()))]))
    });
    let display_scale = std::rc::Rc::new(display_scale);
    host.async_action("pdf.page", move |arg| {
        let display_scale = display_scale.clone();
        async move {
            let path = path_of(&arg)?;
            let page = arg.field("page").and_then(Value::as_f64).filter(|n| *n >= 1.).ok_or("no page")? as usize;
            let zoom = arg.field("zoom").and_then(Value::as_f64).filter(|z| z.is_finite() && *z > 0.).ok_or("no zoom")?;
            let scale = quantize(zoom * display_scale().max(1.));
            let out = cache_dir().join(format!("{}.png", key(&path, stamp(&path)?, page, scale)));
            if !out.is_file() {
                let (reply, mut answer) = zgx::task::latest();
                submit(Job::Page { path, page, scale, out: out.clone(), reply });
                answer.recv().await.ok_or(DAMAGED)??;
            }
            used(&out);
            Ok(Value::record([("src", Value::from(out.to_string_lossy().as_ref()))]))
        }
    });
}

const DAMAGED: &str = "It's damaged, or not a PDF.";
const LOCKED: &str = "It's locked with a password.";
const EMPTY: &str = "It has no pages.";

fn path_of(arg: &Value) -> Result<PathBuf, String> {
    let path = arg.field("path").map(crate::auth::text).unwrap_or_default();
    if path.is_empty() || !Path::new(&path).is_absolute() {
        return Err("That PDF isn't a file on this computer".into());
    }
    Ok(PathBuf::from(path))
}

fn quantize(scale: f64) -> f64 {
    (scale * 100.).round().max(1.) / 100.
}

type Stamp = (u128, u64);

fn stamp(path: &Path) -> Result<Stamp, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    Ok((modified, meta.len()))
}

fn key(path: &Path, stamp: Stamp, page: usize, scale: f64) -> String {
    let mut hash = Sha256::new();
    hash.update(format!("{STYLE_VERSION}\0{}\0{}\0{}\0{page}\0{scale:.2}", path.display(), stamp.0, stamp.1));
    hash.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn cache_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let id = objc2_foundation::NSBundle::mainBundle()
            .bundleIdentifier()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "zeron-zgx".into());
        let caches = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches")).unwrap_or_else(std::env::temp_dir);
        caches.join(id).join("pdf")
    })
    .clone()
}

/// The cache drops the least recently served first; only the access time moves, since a shown picture's file edited reloads the scripts.
fn used(out: &Path) {
    if let Ok(file) = std::fs::File::open(out) {
        let _ = file.set_times(std::fs::FileTimes::new().set_accessed(std::time::SystemTime::now()));
    }
}

enum Job {
    Open { path: PathBuf, reply: zgx::task::LatestSender<Result<Vec<(f64, f64)>, String>> },
    Page { path: PathBuf, page: usize, scale: f64, out: PathBuf, reply: zgx::task::LatestSender<Result<(), String>> },
}

struct Queue {
    jobs: Mutex<Vec<Job>>,
    ready: Condvar,
}

fn submit(job: Job) {
    static QUEUE: OnceLock<Arc<Queue>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let queue = Arc::new(Queue { jobs: Mutex::new(Vec::new()), ready: Condvar::new() });
        let worker = queue.clone();
        std::thread::Builder::new().name("zeron-pdf".into()).spawn(move || work(&worker)).expect("the PDF thread");
        queue
    });
    queue.jobs.lock().unwrap_or_else(|e| e.into_inner()).push(job);
    queue.ready.notify_one();
}

fn work(queue: &Queue) {
    let mut open: Vec<(PathBuf, Stamp, Document)> = Vec::new();
    loop {
        let job = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = jobs.pop() {
                    break job;
                }
                jobs = queue.ready.wait(jobs).unwrap_or_else(|e| e.into_inner());
            }
        };
        // A panicking page must not end the thread every later request waits on.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
            Job::Open { path, reply } => {
                reply.send(document(&mut open, &path).and_then(|doc| doc.sizes()));
            }
            Job::Page { path, page, scale, out, reply } => {
                let done = if out.is_file() {
                    Ok(())
                } else {
                    document(&mut open, &path).and_then(|doc| {
                        let dir = out.parent().unwrap_or(Path::new("."));
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                        doc.render(page, scale, &out)?;
                        prune(dir, CACHE_BYTES);
                        Ok(())
                    })
                };
                reply.send(done);
            }
        }));
    }
}

fn document<'a>(open: &'a mut Vec<(PathBuf, Stamp, Document)>, path: &Path) -> Result<&'a Document, String> {
    let now = stamp(path)?;
    if let Some(i) = open.iter().position(|(p, s, _)| p == path && *s == now) {
        let kept = open.remove(i);
        open.push(kept);
    } else {
        open.retain(|(p, _, _)| p != path);
        let doc = Document::open(path)?;
        open.push((path.to_path_buf(), now, doc));
        if open.len() > KEEP_OPEN {
            open.remove(0);
        }
    }
    Ok(&open.last().expect("just kept").2)
}

struct Document(DocRef);

impl Drop for Document {
    fn drop(&mut self) {
        unsafe { CGPDFDocumentRelease(self.0) }
    }
}

impl Document {
    fn open(path: &Path) -> Result<Document, String> {
        if !path.is_file() {
            return Err("It isn't there anymore.".into());
        }
        let url = CFURL::from_path(path, false).ok_or(DAMAGED)?;
        let doc = unsafe { CGPDFDocumentCreateWithURL(url.as_concrete_TypeRef()) };
        if doc.is_null() {
            return Err(DAMAGED.into());
        }
        let doc = Document(doc);
        if unsafe { CGPDFDocumentIsEncrypted(doc.0) && !CGPDFDocumentIsUnlocked(doc.0) } {
            return Err(LOCKED.into());
        }
        if unsafe { CGPDFDocumentGetNumberOfPages(doc.0) } == 0 {
            return Err(EMPTY.into());
        }
        Ok(doc)
    }

    fn page(&self, number: usize) -> Result<Page, String> {
        let page = unsafe { CGPDFDocumentGetPage(self.0, number) };
        if page.is_null() {
            return Err(format!("It has no page {number}."));
        }
        let crop = unsafe { CGPDFPageGetBoxRect(page, CROP_BOX) };
        let turn = unsafe { CGPDFPageGetRotationAngle(page) }.rem_euclid(360) / 90 * 90;
        Ok(Page { page, crop, turn })
    }

    fn sizes(&self) -> Result<Vec<(f64, f64)>, String> {
        let n = unsafe { CGPDFDocumentGetNumberOfPages(self.0) };
        (1..=n).map(|i| self.page(i).map(|p| p.size())).collect()
    }

    fn render(&self, number: usize, scale: f64, out: &Path) -> Result<(u32, u32), String> {
        let page = self.page(number)?;
        let (w, h) = page.size();
        if !(w > 0. && h > 0.) {
            return Err(DAMAGED.into());
        }
        let s = scale.min(MAX_SIDE / w.max(h)).min((MAX_PIXELS / (w * h)).sqrt());
        let (pw, ph) = ((w * s).round().max(1.) as usize, (h * s).round().max(1.) as usize);
        let space = CGColorSpace::create_device_rgb();
        let mut context = CGContext::create_bitmap_context(None, pw, ph, 8, pw * 4, &space, kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
        context.set_rgb_fill_color(1., 1., 1., 1.);
        context.fill_rect(CGRect::new(&CGPoint::new(0., 0.), &CGSize::new(pw as CGFloat, ph as CGFloat)));
        context.set_interpolation_quality(CGInterpolationQuality::CGInterpolationQualityHigh);
        context.scale(pw as CGFloat / w, ph as CGFloat / h);
        context.concat_ctm(page.turned());
        context.translate(-page.crop.origin.x, -page.crop.origin.y);
        context.clip_to_rect(page.crop);
        unsafe { CGContextDrawPDFPage(context.as_ptr(), page.page) };
        if context.bytes_per_row() != pw * 4 {
            return Err(DAMAGED.into());
        }
        write_png(out, pw as u32, ph as u32, &context.data()[..pw * ph * 4])?;
        Ok((pw as u32, ph as u32))
    }
}

struct Page {
    page: PageRef,
    crop: CGRect,
    turn: i32,
}

impl Page {
    fn size(&self) -> (f64, f64) {
        let (w, h) = (self.crop.size.width, self.crop.size.height);
        if self.turn % 180 == 0 { (w, h) } else { (h, w) }
    }

    fn turned(&self) -> CGAffineTransform {
        let (w, h) = (self.crop.size.width, self.crop.size.height);
        let (a, b, c, d, tx, ty) = match self.turn {
            90 => (0., -1., 1., 0., 0., w),
            180 => (-1., 0., 0., -1., w, h),
            270 => (0., 1., -1., 0., h, 0.),
            _ => (1., 0., 0., 1., 0., 0.),
        };
        CGAffineTransform { a, b, c, d, tx, ty }
    }
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    let part = path.with_extension(format!("{}.part", std::process::id()));
    std::fs::write(&part, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&part, path).map_err(|e| e.to_string())
}

fn prune(dir: &Path, cap: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((meta.accessed().or_else(|_| meta.modified()).unwrap_or(UNIX_EPOCH), meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= cap {
        return;
    }
    files.sort();
    for (_, len, path) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pages: &[(f64, f64, i32)]) -> Vec<u8> {
        let mut objects: Vec<String> = vec![String::new(), String::new()];
        let mut kids = Vec::new();
        for &(w, h, rotate) in pages {
            let content = format!("0 g {} {} {} {} re f", 0, h / 2., w / 2., h / 2.);
            objects.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
            let content_id = objects.len();
            objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Rotate {rotate} /Contents {content_id} 0 R >>"));
            kids.push(format!("{} 0 R", objects.len()));
        }
        objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".into();
        objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), kids.len());
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zgx-pdf-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pixels(path: &Path) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        buf.truncate(info.buffer_size());
        (info.width, info.height, buf)
    }

    fn dark_at(img: &(u32, u32, Vec<u8>), fx: f64, fy: f64) -> bool {
        let (w, h, data) = img;
        let (x, y) = ((fx * *w as f64) as usize, (fy * *h as f64) as usize);
        data[(y * *w as usize + x) * 4] < 64
    }

    #[test]
    fn pdf_counts_its_pages_and_sizes_them_turned() {
        let dir = scratch("count");
        let path = dir.join("three.pdf");
        std::fs::write(&path, sample(&[(612., 792., 0), (612., 792., 90), (300., 200., 0)])).unwrap();
        let doc = Document::open(&path).unwrap();
        assert_eq!(doc.sizes().unwrap(), vec![(612., 792.), (792., 612.), (300., 200.)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_page_renders_at_its_scale_upright_and_turned() {
        let dir = scratch("render");
        let path = dir.join("two.pdf");
        std::fs::write(&path, sample(&[(200., 100., 0), (200., 100., 90)])).unwrap();
        let doc = Document::open(&path).unwrap();

        let upright = dir.join("1.png");
        assert_eq!(doc.render(1, 2., &upright).unwrap(), (400, 200));
        let img = pixels(&upright);
        assert_eq!((img.0, img.1), (400, 200));
        assert!(dark_at(&img, 0.25, 0.25), "the box in the top-left quarter");
        assert!(!dark_at(&img, 0.75, 0.25) && !dark_at(&img, 0.25, 0.75), "white elsewhere");

        let turned = dir.join("2.png");
        assert_eq!(doc.render(2, 3., &turned).unwrap(), (300, 600));
        let img = pixels(&turned);
        assert!(dark_at(&img, 0.75, 0.25), "the box at the top right");
        assert!(!dark_at(&img, 0.25, 0.25) && !dark_at(&img, 0.75, 0.75));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_page_past_the_bounds_is_drawn_at_a_lower_density() {
        let dir = scratch("bounds");
        let path = dir.join("big.pdf");
        std::fs::write(&path, sample(&[(612., 792., 0)])).unwrap();
        let doc = Document::open(&path).unwrap();
        let (w, h) = doc.render(1, 40., &dir.join("big.png")).unwrap();
        assert!(w as f64 <= MAX_SIDE && h as f64 <= MAX_SIDE, "{w}x{h}");
        assert!((w as f64) * (h as f64) <= MAX_PIXELS * 1.01, "{w}x{h}");
        assert!(h > 4000, "still as dense as the bounds allow: {w}x{h}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_cache_key_names_the_file_version_page_and_scale() {
        let p = Path::new("/a/b.pdf");
        let base = key(p, (1, 10), 1, 2.);
        assert_eq!(base, key(p, (1, 10), 1, 2.));
        assert_eq!(base.len(), 32);
        for other in [key(Path::new("/a/c.pdf"), (1, 10), 1, 2.), key(p, (2, 10), 1, 2.), key(p, (1, 11), 1, 2.), key(p, (1, 10), 2, 2.), key(p, (1, 10), 1, 2.5)] {
            assert_ne!(base, other);
        }
        assert_eq!(quantize(1.234), 1.23);
        assert_eq!(key(p, (1, 10), 1, quantize(2.001)), base);
    }

    #[test]
    fn pdf_that_cant_be_shown_says_why() {
        let dir = scratch("bad");
        let junk = dir.join("junk.pdf");
        std::fs::write(&junk, b"%PDF-1.4\nnot really a pdf at all\n").unwrap();
        assert_eq!(Document::open(&junk).err().as_deref(), Some(DAMAGED));

        let empty = dir.join("empty.pdf");
        std::fs::write(&empty, sample(&[])).unwrap();
        // CoreGraphics opens no document without pages: it reads as damaged.
        assert!(matches!(Document::open(&empty).err().as_deref(), Some(EMPTY | DAMAGED)));

        let locked = dir.join("locked.pdf");
        write_locked(&locked);
        assert_eq!(Document::open(&locked).err().as_deref(), Some(LOCKED));

        assert!(Document::open(&dir.join("gone.pdf")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_cache_past_its_cap_drops_the_least_recently_served() {
        let dir = scratch("prune");
        let (old, shown, newest) = (dir.join("old.png"), dir.join("shown.png"), dir.join("newest.png"));
        for (path, age) in [(&shown, 300), (&old, 200), (&newest, 0)] {
            std::fs::write(path, [0u8; 100]).unwrap();
            let when = std::time::SystemTime::now() - std::time::Duration::from_secs(age);
            let times = std::fs::FileTimes::new().set_accessed(when).set_modified(when);
            std::fs::File::options().write(true).open(path).unwrap().set_times(times).unwrap();
        }
        let modified = std::fs::metadata(&shown).unwrap().modified().unwrap();
        used(&shown);
        assert_eq!(std::fs::metadata(&shown).unwrap().modified().unwrap(), modified, "serving leaves the file unedited");
        prune(&dir, 200);
        assert!(!old.exists(), "the least recently served goes");
        assert!(shown.exists() && newest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_encrypted_without_a_password_opens() {
        let dir = scratch("open");
        let path = dir.join("owner-only.pdf");
        write_encrypted(&path, None);
        let doc = Document::open(&path).unwrap();
        assert_eq!(doc.sizes().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn write_locked(path: &Path) {
        write_encrypted(path, Some("secret"));
    }

    fn write_encrypted(path: &Path, user: Option<&str>) {
        use core_foundation::{dictionary::CFDictionary, string::CFString};
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            static kCGPDFContextUserPassword: core_foundation::string::CFStringRef;
            static kCGPDFContextOwnerPassword: core_foundation::string::CFStringRef;
            fn CGPDFContextCreateWithURL(url: core_foundation::url::CFURLRef, media: *const CGRect, info: core_foundation::dictionary::CFDictionaryRef) -> *mut core_graphics::sys::CGContext;
            fn CGPDFContextBeginPage(context: *mut core_graphics::sys::CGContext, info: *const c_void);
            fn CGPDFContextEndPage(context: *mut core_graphics::sys::CGContext);
            fn CGPDFContextClose(context: *mut core_graphics::sys::CGContext);
        }
        let mut pairs = vec![(unsafe { CFString::wrap_under_get_rule(kCGPDFContextOwnerPassword) }, CFString::new("owner"))];
        if let Some(user) = user {
            pairs.push((unsafe { CFString::wrap_under_get_rule(kCGPDFContextUserPassword) }, CFString::new(user)));
        }
        let info = CFDictionary::from_CFType_pairs(&pairs);
        let url = CFURL::from_path(path, false).unwrap();
        let media = CGRect::new(&CGPoint::new(0., 0.), &CGSize::new(200., 100.));
        unsafe {
            let raw = CGPDFContextCreateWithURL(url.as_concrete_TypeRef(), &media, info.as_concrete_TypeRef());
            assert!(!raw.is_null());
            let context = CGContext::from_ptr(raw);
            CGPDFContextBeginPage(context.as_ptr(), std::ptr::null());
            context.set_rgb_fill_color(0., 0., 0., 1.);
            context.fill_rect(CGRect::new(&CGPoint::new(10., 10.), &CGSize::new(50., 50.)));
            CGPDFContextEndPage(context.as_ptr());
            CGPDFContextClose(context.as_ptr());
        }
    }
}
