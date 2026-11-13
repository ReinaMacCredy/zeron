//! Voice: Settings → Voice's Codex voice conversation rows, which of your
//! devices can stream voice to Codex there. (Dictation is src/dictation.rs.)
//!
//!   voice.remote({ ids })     { [id] = true } for each of your devices that
//!                             can stream voice to Codex there

use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Value};

use crate::auth::text;

pub fn install(_cx: &mut WindowContext, host: &ScriptHost) {
    host.async_action("voice.remote", |args| {
        let ids: Vec<String> = match args.field("ids") {
            Some(Value::List(list)) => list.iter().map(text).collect(),
            _ => Vec::new(),
        };
        async move {
            let capable = ids
                .iter()
                .filter(|id| crate::sessions::device_supports(id, zeron_proto::voice::remote::CAPABILITY))
                .map(|id| (id.as_str(), Value::Bool(true)))
                .collect::<Vec<_>>();
            Ok(Value::record(capable))
        }
    });
}
