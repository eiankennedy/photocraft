//! PhotoCraft hosted inside Workbench's side panel: the same app as the desktop build, drawn into a view
//! Workbench owns, with Workbench's look (and no Hands off: a photo editor keeps its panels). Workbench (and
//! Claude) drive it through its control channel. See `craft-embed`.
//!
//! The desktop build's `eframe::App` impl is driven as eframe drives it, with an [`eframe::Frame`] that carries
//! the host's GPU (the canvas and compositor draw with wgpu directly, as egui paint callbacks).

use craft_embed::{EmbedApp, egui, egui_wgpu};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::control::{self, ControlRequest, Outcome};
use photocraft_ui_egui::{ExportSettings, FileStamp, PhotocraftApp, Services};
use serde_json::{Value, json};

struct Photo {
    app: PhotocraftApp,
    frame: eframe::Frame,
    ctx: egui::Context,
}

impl EmbedApp for Photo {
    fn logic(&mut self, ctx: &egui::Context) {
        eframe::App::logic(&mut self.app, ctx, &mut self.frame);
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        eframe::App::raw_input_hook(&mut self.app, ctx, raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui) {
        eframe::App::ui(&mut self.app, ui, &mut self.frame);
    }
    fn control(&mut self, ctx: &egui::Context, request: &str) -> String {
        let req: Value = serde_json::from_str(request).unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));
        // a photo editor has no Hands off (E's rule): Workbench's call is taken and nothing changes
        if hands_off(method, &params) {
            return json!({"ok": true, "result": {"on": false}}).to_string();
        }
        let (method, params) = translate(method, params);
        let (req, _reply) = ControlRequest::new(&method, params);
        let reply = match control::handle(&mut self.app, ctx, &req) {
            Outcome::Done(v) => v,
            Outcome::Screenshot { .. } => json!({"ok": false, "error": "Workbench takes the screenshot of the panel itself"}),
            // Both settle over later frames, which run after this call returns.
            Outcome::AfterInput => json!({"ok": true, "result": {"pending": "input"}}),
            Outcome::AfterJob(job) => json!({"ok": true, "result": {"job": job.0, "pending": true}}),
        };
        reply.to_string()
    }
    fn gpu(&mut self, rs: &egui_wgpu::RenderState) {
        // What the desktop's app creator does with `cc.wgpu_render_state`: the GPU canvas and compositor.
        self.frame.wgpu_render_state = Some(rs.clone());
        self.app.set_wgpu(rs.clone());
        self.app.start_compositor(&self.ctx);
    }
    fn exit(&mut self) {
        eframe::App::on_exit(&mut self.app);
    }
}

/// Whether a request is `view.handsOff`, which the panel takes without hiding anything.
fn hands_off(method: &str, params: &Value) -> bool {
    let command = params.get("command").or_else(|| params.get("id")).and_then(Value::as_str);
    method == "view.handsOff" || (matches!(method, "engine.execute" | "command" | "ui.command") && command == Some("view.handsOff"))
}

/// The protocol's method and params as PhotoCraft's control channel names them: `document.inspect` is an engine
/// command there.
fn translate(method: &str, params: Value) -> (String, Value) {
    match method {
        "document.inspect" => ("engine.execute".into(), json!({"command": "document.inspect", "params": params})),
        other => (other.into(), params),
    }
}

/// Files by path: read by the app itself, decoded, encoded and written here (no dialogs, no prefs file,
/// no recovery folder: Workbench owns the files).
fn services() -> Services {
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &ExportSettings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        write: Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
        file_stamp: Some(Box::new(|path: &str| {
            let meta = std::fs::metadata(path).ok()?;
            Some(FileStamp { len: meta.len(), modified: meta.modified().ok() })
        })),
        ..Default::default()
    }
}

fn make(ctx: &egui::Context) -> Box<dyn EmbedApp> {
    PhotocraftApp::install_type_reset_hooks(ctx);
    let mut app = PhotocraftApp::new(Session::new(), services());
    // Long commands and file opens run as background jobs with progress and Cancel, as on the desktop.
    app.background_jobs = true;
    // Workbench's look from the first frame (Workbench sets the mode's accent right after); no Hands off in a
    // photo editor
    let _ = photocraft_ui_egui::menus::invoke(&mut app, ctx, "view.workbench", json!({"on": true}));
    Box::new(Photo { app, frame: eframe::Frame::_new_kittest(), ctx: ctx.clone() })
}

craft_embed::export_embed!(make);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_off_is_taken_and_document_inspect_is_the_engine_command() {
        assert!(hands_off("engine.execute", &json!({"command": "view.handsOff", "params": {"on": true}})));
        assert!(!hands_off("engine.execute", &json!({"command": "view.workbench"})));
        let (m, p) = translate("document.inspect", json!({}));
        assert_eq!((m.as_str(), p), ("engine.execute", json!({"command": "document.inspect", "params": {}})));
        assert_eq!(translate("ui.inspect", json!({})).0, "ui.inspect");
    }
}
