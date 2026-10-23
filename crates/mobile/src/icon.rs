use resvg::{tiny_skia, usvg};

#[uniffi::export]
pub fn svg_icon_png(svg: Vec<u8>, side: u32) -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let size = tree.size();
    let scale = side as f32 / size.width().max(size.height());
    let dx = (side as f32 - size.width() * scale) / 2.0;
    let dy = (side as f32 - size.height() * scale) / 2.0;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn rasterizes_a_square_icon_and_rejects_garbage() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="#000"/></svg>"##;
        let png = super::svg_icon_png(svg.to_vec(), 48).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(super::svg_icon_png(b"not svg".to_vec(), 48).is_none());
    }
}
