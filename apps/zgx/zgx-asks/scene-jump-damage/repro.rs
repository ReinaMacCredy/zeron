use zgx::{compose::prelude::*, scene::Rect, widgets::Ui};

struct Lab {
    ui: Ui,
    offset: zgx::reactive::Signal<f32>,
    shown: zgx::reactive::Signal<bool>,
}

fn lab() -> Lab {
    let mut ui = Ui::new(300., 300.);
    let offset = ui.signal(0.);
    let shown = ui.signal(true);
    let read_shown = shown.clone();
    ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(200.)
            .child(div().h(100.))
            .child(switch(
                move || read_shown.get(),
                |shown, _| match shown {
                    true => div().w(60.).h(16.).bg(rgb(0xff2020)),
                    false => div().h(16.),
                },
            ))
            .child(div().h(2000.)),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    Lab { ui, offset, shown }
}

fn uncovered(damage: &[Rect], painted: Rect) -> usize {
    let mut n = 0;
    for y in 0..painted.height as i32 {
        for x in 0..painted.width as i32 {
            let pixel = Rect::new(painted.x + x as f32, painted.y + y as f32, 1., 1.);
            if !damage.iter().any(|rect| rect.intersects(pixel)) {
                n += 1;
            }
        }
    }
    n
}

const LANE: Rect = Rect { x: 0., y: 100., width: 60., height: 16. };

#[test]
fn removed_after_a_jump_past_the_view() {
    let mut l = lab();
    l.offset.set(600.);
    l.ui.prepare_frame();
    l.shown.set(false);
    l.ui.prepare_frame();
    let damage = l.ui.scene.borrow_mut().flush().damage;
    println!("after-jump damage={damage:?} uncovered={}", uncovered(&damage, LANE));
    assert_eq!(uncovered(&damage, LANE), 0, "stale pixels: {damage:?}");
}

#[test]
fn removed_before_a_jump_past_the_view() {
    let mut l = lab();
    l.shown.set(false);
    l.ui.prepare_frame();
    l.offset.set(600.);
    l.ui.prepare_frame();
    let damage = l.ui.scene.borrow_mut().flush().damage;
    println!("before-jump damage={damage:?} uncovered={}", uncovered(&damage, LANE));
    assert_eq!(uncovered(&damage, LANE), 0, "stale pixels: {damage:?}");
}

#[test]
fn removed_one_flush_after_the_jump() {
    let mut l = lab();
    l.offset.set(600.);
    l.ui.prepare_frame();
    let first = l.ui.scene.borrow_mut().flush().damage;
    l.shown.set(false);
    l.ui.prepare_frame();
    let second = l.ui.scene.borrow_mut().flush().damage;
    println!("next-flush first={first:?} second={second:?} uncovered={}", uncovered(&first, LANE));
    assert_eq!(uncovered(&first, LANE), 0, "stale pixels: {first:?}");
}

#[test]
fn removed_after_a_small_move_stays_in_the_view() {
    let mut l = lab();
    l.offset.set(30.);
    l.ui.prepare_frame();
    l.shown.set(false);
    l.ui.prepare_frame();
    let report = l.ui.scene.borrow_mut().flush();
    println!("small-move damage={:?} moves={:?}", report.damage, l.ui.scene.borrow().scroll_moves());
    let moved = Rect { y: 70., ..LANE };
    assert_eq!(uncovered(&report.damage, moved), 0, "stale pixels: {:?}", report.damage);
}

fn shrink_lab() -> (Ui, zgx::reactive::Signal<f32>, zgx::reactive::Signal<bool>) {
    let mut ui = Ui::new(300., 300.);
    let offset = ui.signal(0.);
    let wide = ui.signal(true);
    let read = wide.clone();
    ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(200.)
            .child(div().h(100.))
            .child(
                div()
                    .h(16.)
                    .reactive_style(move || Styles::new().w(if read.get() { 120. } else { 40. }))
                    .bg(rgb(0x2020ff)),
            )
            .child(div().h(2000.)),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    (ui, offset, wide)
}

#[test]
fn shrunk_after_a_jump_past_the_view() {
    let (mut ui, offset, wide) = shrink_lab();
    offset.set(600.);
    ui.prepare_frame();
    wide.set(false);
    ui.prepare_frame();
    let damage = ui.scene.borrow_mut().flush().damage;
    let bar = Rect { x: 0., y: 100., width: 120., height: 16. };
    println!("shrunk damage={damage:?} uncovered={}", uncovered(&damage, bar));
    assert_eq!(uncovered(&damage, bar), 0, "stale pixels: {damage:?}");
}
