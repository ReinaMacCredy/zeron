use std::path::PathBuf;

pub fn bundled_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe
        .parent()?
        .parent()?
        .join("Resources")
        .join("cua-driver")
        .join("cua-driver");
    path.is_file().then_some(path)
}
