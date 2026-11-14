use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
};

const SP_PER_BP: f64 = 65781.76;
const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
struct Node {
    kind: u8,
    tag: u32,
    line: u32,
    h: f64,
    v: f64,
    w: f64,
    ht: f64,
    dp: f64,
    parent: Option<usize>,
}

impl Node {
    fn is_leaf(&self) -> bool {
        matches!(self.kind, b'g' | b'k' | b'x' | b'$' | b'h')
    }

    fn rank(&self) -> u8 {
        match self.kind {
            b'g' | b'$' => 0,
            b'k' => 1,
            _ => 2,
        }
    }

    fn left(&self) -> f64 {
        self.h.min(self.h + self.w)
    }

    fn right(&self) -> f64 {
        self.h.max(self.h + self.w)
    }

    fn holds(&self, x: f64, y: f64) -> bool {
        self.ht + self.dp > 0. && x >= self.left() && x <= self.right() && y >= self.v - self.ht && y <= self.v + self.dp
    }

    fn distance(&self, x: f64, y: f64) -> (f64, f64) {
        let dy = if y < self.v - self.ht { self.v - self.ht - y } else if y > self.v + self.dp { y - self.v - self.dp } else { 0. };
        let dx = if x < self.left() { self.left() - x } else if x > self.right() { x - self.right() } else { 0. };
        (dy, dx)
    }
}

#[derive(Debug, PartialEq)]
pub struct Spot {
    pub file: PathBuf,
    pub line: u32,
    pub end: u32,
}

#[derive(Default)]
pub struct Synctex {
    inputs: HashMap<u32, String>,
    pages: HashMap<u32, Vec<Node>>,
}

pub fn read(path: &Path) -> Result<Synctex, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = if bytes.starts_with(&[0x1f, 0x8b]) { gunzip(&bytes)? } else { bytes };
    Ok(parse(&String::from_utf8_lossy(&text)))
}

fn gunzip(data: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "a damaged SyncTeX file".to_string();
    if data.len() < 18 || data[2] != 8 {
        return Err(bad());
    }
    let flags = data[3];
    let mut at = 10;
    if flags & 4 != 0 {
        let extra = *data.get(at).ok_or_else(bad)? as usize | (*data.get(at + 1).ok_or_else(bad)? as usize) << 8;
        at += 2 + extra;
    }
    for bit in [8, 16] {
        if flags & bit != 0 {
            at += data.get(at..).ok_or_else(bad)?.iter().position(|b| *b == 0).ok_or_else(bad)? + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    miniz_oxide::inflate::decompress_to_vec_with_limit(data.get(at..).ok_or_else(bad)?, MAX_BYTES).map_err(|_| bad())
}

fn numbers<const N: usize>(field: &str) -> Option<[f64; N]> {
    let mut out = [0.; N];
    let mut parts = field.split(',');
    for slot in out.iter_mut() {
        *slot = parts.next()?.trim().parse().ok()?;
    }
    Some(out)
}

pub fn parse(text: &str) -> Synctex {
    let mut out = Synctex::default();
    let (mut unit, mut magnification, mut x_offset, mut y_offset) = (1., 1000., 0., 0.);
    let mut raw: Vec<(u32, Node)> = Vec::new();
    let mut page: Option<u32> = None;
    let mut stack: Vec<usize> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Input:") {
            if let Some((tag, path)) = rest.split_once(':')
                && let Ok(tag) = tag.parse()
            {
                out.inputs.insert(tag, path.to_string());
            }
            continue;
        }
        if page.is_none() {
            let header = |key: &str| line.strip_prefix(key).and_then(|v| v.trim().parse::<f64>().ok());
            if let Some(v) = header("Unit:") {
                unit = v;
            } else if let Some(v) = header("Magnification:") {
                magnification = v;
            } else if let Some(v) = header("X Offset:") {
                x_offset = v;
            } else if let Some(v) = header("Y Offset:") {
                y_offset = v;
            } else if let Some(n) = line.strip_prefix('{').and_then(|n| n.parse().ok()) {
                page = Some(n);
                stack.clear();
            } else if line.starts_with("Postamble:") {
                break;
            }
            continue;
        }
        let Some(&kind) = line.as_bytes().first() else { continue };
        match kind {
            b'}' => page = None,
            b']' | b')' => {
                stack.pop();
            }
            b'[' | b'(' | b'h' | b'v' | b'r' | b'k' | b'g' | b'$' | b'x' => {
                let mut fields = line[1..].split(':');
                let (Some(at), Some(place)) = (fields.next(), fields.next()) else { continue };
                let mut at = at.split(',');
                let (Some(Ok(tag)), Some(Ok(source_line))) = (at.next().map(str::parse), at.next().map(str::parse)) else { continue };
                let Some([h, v]) = numbers::<2>(place) else { continue };
                let size = fields.next();
                let (w, ht, dp) = match kind {
                    b'k' => (size.and_then(numbers::<1>).map_or(0., |[w]| w), 0., 0.),
                    b'g' | b'$' | b'x' => (0., 0., 0.),
                    _ => size.and_then(numbers::<3>).map_or((0., 0., 0.), |[w, ht, dp]| (w, ht, dp)),
                };
                let node = Node { kind, tag, line: source_line, h, v, w, ht, dp, parent: stack.last().copied() };
                let Some(p) = page else { continue };
                raw.push((p, node));
                if kind == b'[' || kind == b'(' {
                    stack.push(raw.len() - 1);
                }
            }
            _ => {}
        }
    }
    let scale = unit * magnification / 1000. / SP_PER_BP;
    let (dx, dy) = (x_offset * unit / SP_PER_BP, y_offset * unit / SP_PER_BP);
    let mut index: Vec<(u32, usize)> = Vec::with_capacity(raw.len());
    for (p, node) in &raw {
        let list = out.pages.entry(*p).or_default();
        index.push((*p, list.len()));
        list.push(Node {
            h: node.h * scale + dx,
            v: node.v * scale + dy,
            w: node.w * scale,
            ht: node.ht * scale,
            dp: node.dp * scale,
            parent: None,
            ..*node
        });
    }
    for (i, (p, node)) in raw.iter().enumerate() {
        if let Some(parent) = node.parent
            && index[parent].0 == *p
        {
            let at = index[i].1;
            out.pages.get_mut(p).expect("indexed")[at].parent = Some(index[parent].1);
        }
    }
    out
}

pub(super) fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

impl Synctex {
    fn file(&self, tag: u32, base: &Path) -> Option<PathBuf> {
        let name = self.inputs.get(&tag)?;
        if name.is_empty() {
            return None;
        }
        let path = Path::new(name);
        Some(normalized(&if path.is_absolute() { path.to_path_buf() } else { base.join(path) }))
    }

    fn inside(nodes: &[Node], mut i: usize, ancestor: usize) -> bool {
        while let Some(p) = nodes[i].parent {
            if p == ancestor {
                return true;
            }
            i = p;
        }
        false
    }

    pub fn spot(&self, page: u32, x: f64, y: f64, base: &Path, mine: impl Fn(&Path) -> bool) -> Option<Spot> {
        let nodes = self.pages.get(&page)?;
        let boxes = nodes.iter().enumerate().filter(|(_, n)| n.kind == b'(');
        let held = boxes.clone().filter(|(_, n)| n.holds(x, y)).min_by(|a, b| {
            let area = |n: &Node| (n.right() - n.left()) * (n.ht + n.dp);
            area(a.1).total_cmp(&area(b.1))
        });
        let at = held.or_else(|| {
            boxes.filter(|(_, n)| n.ht + n.dp > 0.).min_by(|a, b| {
                let (ay, ax) = a.1.distance(x, y);
                let (by, bx) = b.1.distance(x, y);
                ay.total_cmp(&by).then(ax.total_cmp(&bx))
            })
        });
        let (owner, _) = at?;
        let mut leaves: Vec<&Node> = nodes.iter().enumerate().filter(|(i, n)| n.is_leaf() && Self::inside(nodes, *i, owner)).map(|(_, n)| n).collect();
        let behind = |n: &Node| if n.h <= x { (n.rank(), 0, x - n.h) } else { (n.rank(), 1, n.h - x) };
        leaves.sort_by(|a, b| {
            let (ra, ka, da) = behind(a);
            let (rb, kb, db) = behind(b);
            ra.cmp(&rb).then(ka.cmp(&kb)).then(da.total_cmp(&db))
        });
        let mut chain = vec![&nodes[owner]];
        let mut up = nodes[owner].parent;
        while let Some(p) = up {
            chain.push(&nodes[p]);
            up = nodes[p].parent;
        }
        leaves.into_iter().chain(chain).find_map(|n| {
            let file = self.file(n.tag, base)?;
            (n.line > 0 && mine(&file)).then_some(Spot { file, line: n.line, end: n.line })
        })
    }

    pub fn span(&self, page: u32, from: (f64, f64), to: (f64, f64), base: &Path, mine: impl Fn(&Path) -> bool) -> Option<Spot> {
        let nodes = self.pages.get(&page)?;
        let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
        let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
        let tall = |n: &Node| n.parent.map_or((0., 0.), |p| (nodes[p].ht, nodes[p].dp));
        let inside: Vec<&Node> = nodes
            .iter()
            .filter(|n| {
                let (ht, dp) = tall(n);
                n.is_leaf() && n.line > 0 && n.h >= x0 && n.h <= x1 && n.v - ht <= y1 && n.v + dp >= y0
            })
            .collect();
        let best = inside.iter().map(|n| n.rank()).min().unwrap_or(0);
        let mut found: HashMap<u32, (usize, u32, u32)> = HashMap::new();
        for n in inside.into_iter().filter(|n| n.rank() == best) {
            let e = found.entry(n.tag).or_insert((0, n.line, n.line));
            e.0 += 1;
            e.1 = e.1.min(n.line);
            e.2 = e.2.max(n.line);
        }
        let mut tags: Vec<(u32, (usize, u32, u32))> = found.into_iter().collect();
        tags.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        tags.into_iter()
            .find_map(|(tag, (_, line, end))| {
                let file = self.file(tag, base)?;
                mine(&file).then_some(Spot { file, line, end })
            })
            .or_else(|| self.spot(page, (x0 + x1) / 2., (y0 + y1) / 2., base, mine))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "SyncTeX Version:1
Input:1:/doc/./main.tex
Input:2:/tex/texmf-dist/article.cls
Input:3:
Output:pdf
Magnification:1000
Unit:1
X Offset:0
Y Offset:0
Content:
!100
{1
[1,30:0,0:40000000,50000000,0
(1,12:6578176,6578176:13156352,657817,65781
g1,10:6578176,6578176
x1,12:7894000,6578176
g1,11:9867264,6578176
g1,11:13156352,6578176
)
(1,20:6578176,13156352:13156352,657817,65781
k2,5:6578176,13156352:100
g3,1:9867264,13156352
)
]
}1
Input:4:/doc/out/main.aux
Postamble:
Count:8
";

    fn mine(p: &Path) -> bool {
        p.starts_with("/doc")
    }

    #[test]
    fn synctex_click_takes_the_nearest_record_before_it_in_the_box_under_it() {
        let s = parse(SAMPLE);
        let base = Path::new("/doc");
        let at = |x: f64, y: f64| s.spot(1, x, y, base, mine).map(|s| s.line);
        assert_eq!(at(101., 99.), Some(10), "past the first glue");
        assert_eq!(at(130., 99.), Some(10), "a current-position record is taken after glue");
        assert_eq!(at(151., 99.), Some(11), "past the second glue, before the third");
        assert_eq!(at(250., 99.), Some(11), "the last record behind it");
        assert_eq!(at(101., 300.), Some(20), "no box under it: the nearest, past records outside the project");
        assert_eq!(s.spot(1, 101., 99., base, mine).unwrap().file, PathBuf::from("/doc/main.tex"));
    }

    #[test]
    fn synctex_drag_spans_the_lines_inside_it() {
        let s = parse(SAMPLE);
        let spot = s.span(1, (90., 90.), (210., 110.), Path::new("/doc"), mine).unwrap();
        assert_eq!((spot.line, spot.end), (10, 11));
        assert_eq!(s.span(2, (0., 0.), (1., 1.), Path::new("/doc"), mine), None);
    }

    #[test]
    fn synctex_relative_inputs_resolve_against_the_folder_it_ran_in() {
        let s = parse(&SAMPLE.replace("/doc/./main.tex", "./parts/../main.tex"));
        assert_eq!(s.file(1, Path::new("/doc")), Some(PathBuf::from("/doc/main.tex")));
        assert_eq!(s.file(3, Path::new("/doc")), None);
    }

    #[test]
    fn synctex_reads_gzip_and_plain() {
        let dir = std::env::temp_dir().join(format!("zgx-synctex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("a.synctex");
        std::fs::write(&plain, SAMPLE).unwrap();
        let deflated = miniz_oxide::deflate::compress_to_vec(SAMPLE.as_bytes(), 6);
        let mut gz = vec![0x1f, 0x8b, 8, 8, 0, 0, 0, 0, 0, 3];
        gz.extend_from_slice(b"a.synctex\0");
        gz.extend_from_slice(&deflated);
        gz.extend_from_slice(&[0; 8]);
        let packed = dir.join("a.synctex.gz");
        std::fs::write(&packed, gz).unwrap();
        for path in [plain, packed] {
            assert_eq!(read(&path).unwrap().spot(1, 101., 99., Path::new("/doc"), mine).map(|s| s.line), Some(10));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
