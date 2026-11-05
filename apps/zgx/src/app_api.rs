use std::path::{Path, PathBuf};

use zeron_proto::ui_driver::{
    UiDriverBuild, UiDriverDescriptor, UiDriverEndpoint, UiDriverProtocol, descriptor_path, run_dir,
};

pub struct Published(Vec<PathBuf>);

impl Drop for Published {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn publish(socket: &Path) -> Option<Published> {
    match write(socket) {
        Ok(published) => Some(published),
        Err(error) => {
            eprintln!("zeron-zgx: app api: {error}");
            None
        }
    }
}

fn write(socket: &Path) -> std::io::Result<Published> {
    let (ipc_port, data_dir) = crate::engine::get()
        .ipc_endpoint()
        .ok_or_else(|| std::io::Error::other("no engine config"))?;
    let pid = std::process::id();
    let record = UiDriverDescriptor {
        pid,
        endpoint: UiDriverEndpoint::Unix { socket: socket.to_path_buf() },
        label: std::env::var("ZERON_DEV_APP_NAME").unwrap_or_else(|_| "Zeron Beta Dev".into()),
        bundle_path: None,
        data_dir,
        ipc_port,
        build: if cfg!(debug_assertions) { UiDriverBuild::Debug } else { UiDriverBuild::Release },
        input: true,
        version: crate::about::APP_VERSION.to_owned(),
        started_at: std::time::SystemTime::now().into(),
        protocol: UiDriverProtocol::Zgx,
    };
    let bytes = serde_json::to_vec_pretty(&record)?;
    let mut dirs = vec![run_dir()];
    dirs.extend(crate::dev::smoke_switch("ZERON_ZGX_SHELL_RUN_DIR").map(PathBuf::from));
    dirs.dedup();
    let mut published = Published(Vec::new());
    for dir in dirs {
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = descriptor_path(&dir, pid);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        published.0.push(path);
    }
    Ok(published)
}
