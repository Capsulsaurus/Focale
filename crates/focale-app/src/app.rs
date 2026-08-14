//! The Focale application: session, editor, filmstrip, status bar.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage, Key, RichText, TextureHandle};
use focale_core::color::{Gamut, luminance_rec2020, map_to_gamut, srgb_encode};
use focale_core::params::EditState;
use focale_core::pipeline::RenderWarning;
use focale_sidecar::schema::Flag;
use focale_sidecar::{SidecarDoc, SidecarError};

use crate::export_queue::{self, ExportItem, ExportStatus};
use crate::jobs::{JobHandle, Priority, Scheduler};
use crate::panels;
use crate::preview::{self, PreviewBase, PreviewFrame};
use crate::session::Session;
use crate::suggest::{self, SuggestionSet};
use crate::thumbs;
use crate::viewport::{self, ViewportCallback, ViewportRenderer};

/// Worker → UI messages.
enum Msg {
    Base(PathBuf, Result<PreviewBase, String>),
    AiMask(PathBuf, Result<focale_core::masks::ResolvedMask, String>),
    Frame(PathBuf, Result<Box<PreviewFrame>, String>),
    Thumb(PathBuf, ColorImage),
    Export(usize, ExportStatus),
    Suggest(PathBuf, SuggestionSet),
}

/// Active viewport tool. Pan/zoom is always available via wheel + middle
/// drag; these change what a primary drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pan,
    Crop,
    /// Click to segment the object under the cursor (MobileSAM).
    AiObject,
    LinearMask,
    RadialMask,
    BrushMask,
    Heal,
    Clone,
}

/// The application state.
pub struct FocaleApp {
    scheduler: Scheduler,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    session: Session,
    /// Sidecar documents by raw path (created on first edit).
    docs: HashMap<PathBuf, SidecarDoc>,
    /// Paths whose sidecar needs saving, with the time of last change.
    dirty: HashMap<PathBuf, Instant>,

    /// Decoded preview bases (small cache).
    bases: HashMap<PathBuf, PreviewBase>,
    base_order: Vec<PathBuf>,
    decoding: HashSet<PathBuf>,
    /// Latest rendered preview for the primary image (CPU copy for cursor
    /// probing; the GPU texture is uploaded from it).
    frame: Option<PreviewFrame>,
    frame_uploaded: u64,
    render_pending: Option<JobHandle>,
    render_version: u64,
    warnings: Vec<RenderWarning>,
    /// Last pipeline render failure for the primary image (e.g. a sidecar
    /// stamped with a pipeline version this build does not implement).
    render_error: Option<String>,
    /// Sidecars that exist on disk but failed to load (e.g. written by a
    /// newer schema). Never saved, so Focale cannot clobber a newer file.
    unloadable: HashSet<PathBuf>,

    /// Filmstrip thumbnails.
    thumbs: HashMap<PathBuf, TextureHandle>,
    thumbs_requested: HashSet<PathBuf>,

    /// Active rendering gamut (status-bar key, docs/subsystems/color.md).
    gamut: Gamut,
    tool: Tool,
    /// Zoom: None = fit to window; Some(z) = z× (1.0 = 100%).
    zoom: Option<f32>,
    /// Pan offset in image uv units (centre-anchored).
    pan: egui::Vec2,
    /// Brush radius for mask painting (normalized to long edge).
    brush_radius: f32,
    /// In-progress brush stroke points.
    brush_points: Vec<[f32; 2]>,
    /// In-progress drag anchor (crop/linear/radial/heal source pick).
    drag_start: Option<[f32; 2]>,
    /// Cursor colour probe (working-space linear + display values).
    cursor_probe: Option<([f32; 3], [u8; 3])>,

    exports: Vec<ExportItem>,
    selected_recipe: usize,
    /// Copied settings buffer (batch paste, docs/subsystems/app.md).
    clipboard: Option<EditState>,
    suggestions: SuggestionSet,
    suggest_pending: Option<JobHandle>,
    /// Local ONNX segmentation (docs/subsystems/masks.md); models load lazily from the user
    /// data dir. `None` until first use.
    segmenter: std::sync::Arc<std::sync::Mutex<focale_segment::Segmenter>>,
    segmenting: bool,
}

impl FocaleApp {
    /// Creates the app and registers the viewport renderer.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("Focale requires the wgpu backend");
        render_state
            .renderer
            .write()
            .callback_resources
            .insert(ViewportRenderer::new(render_state));
        let workers = std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(2).max(2))
            .unwrap_or(4);
        let (tx, rx) = channel();
        Self {
            scheduler: Scheduler::new(workers),
            tx,
            rx,
            session: Session::default(),
            docs: HashMap::new(),
            dirty: HashMap::new(),
            bases: HashMap::new(),
            base_order: Vec::new(),
            decoding: HashSet::new(),
            frame: None,
            frame_uploaded: 0,
            render_pending: None,
            render_version: 0,
            warnings: Vec::new(),
            render_error: None,
            unloadable: HashSet::new(),
            thumbs: HashMap::new(),
            thumbs_requested: HashSet::new(),
            gamut: Gamut::Srgb,
            tool: Tool::Pan,
            zoom: None,
            pan: egui::Vec2::ZERO,
            brush_radius: 0.03,
            brush_points: Vec::new(),
            drag_start: None,
            cursor_probe: None,
            exports: Vec::new(),
            selected_recipe: 0,
            clipboard: None,
            suggestions: SuggestionSet::default(),
            suggest_pending: None,
            segmenter: std::sync::Arc::new(std::sync::Mutex::new(focale_segment::Segmenter::new(
                focale_segment::ModelPaths::user_default(),
            ))),
            segmenting: false,
        }
    }

    fn open_directory(&mut self, dir: PathBuf) {
        match Session::open(&dir) {
            Ok(s) => {
                self.session = s;
                self.docs.clear();
                self.dirty.clear();
                self.bases.clear();
                self.base_order.clear();
                self.frame = None;
                self.render_error = None;
                self.unloadable.clear();
                self.thumbs.clear();
                self.thumbs_requested.clear();
                self.request_primary_preview();
            }
            Err(e) => {
                tracing::error!("failed to open directory: {e}");
            }
        }
    }

    /// The sidecar doc for a path, created with defaults when absent. A
    /// sidecar that exists but fails to load (newer schema, corruption) is
    /// remembered in `unloadable` and never saved back — defaults are used
    /// in memory, but the file on disk is left untouched.
    fn doc_mut(&mut self, path: &std::path::Path) -> &mut SidecarDoc {
        if !self.docs.contains_key(path) {
            let sidecar = focale_sidecar::sidecar_path_for(path);
            let doc = match SidecarDoc::load(&sidecar) {
                Ok(doc) => doc,
                Err(SidecarError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    SidecarDoc::new_default(focale_core::PIPELINE_VERSION)
                }
                Err(e) => {
                    tracing::error!(
                        "sidecar load failed for {} ({e}); editing in memory only, \
                         the file on disk will not be overwritten",
                        sidecar.display()
                    );
                    self.unloadable.insert(path.to_path_buf());
                    SidecarDoc::new_default(focale_core::PIPELINE_VERSION)
                }
            };
            self.docs.insert(path.to_path_buf(), doc);
        }
        self.docs.get_mut(path).expect("inserted above")
    }

    fn primary_path(&self) -> Option<PathBuf> {
        self.session.primary_entry().map(|e| e.path.clone())
    }

    /// Kicks off decode and/or render for the primary image.
    fn request_primary_preview(&mut self) {
        let Some(path) = self.primary_path() else {
            return;
        };
        // A stale render failure must not outlive the selection it came from.
        self.render_error = None;
        if let Some(base) = self.bases.get(&path).cloned() {
            self.spawn_render(base);
        } else if !self.decoding.contains(&path) {
            self.decoding.insert(path.clone());
            let tx = self.tx.clone();
            let p = path.clone();
            self.scheduler.submit(Priority::Preview, move || {
                let result = preview::build_base(&p).map_err(|e| e.to_string());
                let _ = tx.send(Msg::Base(p, result));
            });
        }
    }

    fn spawn_render(&mut self, base: PreviewBase) {
        if let Some(h) = self.render_pending.take() {
            h.cancel();
        }
        self.render_version += 1;
        let version = self.render_version;
        // Force-load the doc so the preview uses the sidecar's stored
        // pipeline version, never silently the current one.
        let doc = self.doc_mut(&base.path);
        let pipeline_version = doc.pipeline_version;
        let mut edit = doc.edit.clone();
        // Tools that paint in image coordinates need the un-warped frame.
        if matches!(
            self.tool,
            Tool::Crop
                | Tool::LinearMask
                | Tool::RadialMask
                | Tool::BrushMask
                | Tool::Heal
                | Tool::Clone
        ) {
            edit.geometry.rotate = 0.0;
            edit.geometry.perspective = Default::default();
            edit.geometry.crop = None;
        }
        let tx = self.tx.clone();
        let path = base.path.clone();
        let handle = self.scheduler.submit(Priority::Preview, move || {
            let result = preview::render(&base, &edit, pipeline_version, version)
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Frame(path, result));
        });
        self.render_pending = Some(handle);
    }

    /// Applies the primary image's edit to all selected sidecars (batch
    /// broadcast, docs/subsystems/app.md) and schedules preview + save.
    fn after_edit_change(&mut self) {
        let Some(primary) = self.primary_path() else {
            return;
        };
        let edit = self.doc_mut(&primary).edit.clone();
        let now = Instant::now();
        let selected_paths: Vec<PathBuf> = self
            .session
            .selected
            .iter()
            .filter_map(|&i| self.session.entries.get(i).map(|e| e.path.clone()))
            .collect();
        for p in selected_paths {
            let doc = self.doc_mut(&p);
            doc.edit = edit.clone();
            // Editing never re-stamps `pipeline_version`: each doc keeps
            // rendering with its stored version (even across a mixed-version
            // multi-selection) until the user explicitly upgrades it via the
            // status-bar action.
            self.dirty.insert(p, now);
        }
        self.suggestions = SuggestionSet::default();
        if let Some(base) = self.bases.get(&primary).cloned() {
            self.spawn_render(base);
        }
    }

    /// Saves sidecars whose last change is older than the debounce window.
    fn flush_dirty(&mut self, force: bool) {
        let now = Instant::now();
        let due: Vec<PathBuf> = self
            .dirty
            .iter()
            .filter(|(_, t)| force || now.duration_since(**t) > Duration::from_millis(400))
            .map(|(p, _)| p.clone())
            .collect();
        for path in due {
            self.dirty.remove(&path);
            if self.unloadable.contains(&path) {
                tracing::warn!(
                    "not saving {}: its on-disk sidecar failed to load and must not be clobbered",
                    path.display()
                );
                continue;
            }
            // Keep live-index in sync with session state before saving.
            if let Some(entry) = self.session.entries.iter().find(|e| e.path == path) {
                let live = entry.live.clone();
                let doc = self.doc_mut(&path);
                doc.live_index = live;
            }
            // Debug provenance: which build/OS last wrote this file.
            self.doc_mut(&path)
                .set_provenance(&focale_buildinfo::version(), focale_buildinfo::platform());
            let doc = self.doc_mut(&path).clone();
            let sidecar = focale_sidecar::sidecar_path_for(&path);
            if let Err(e) = doc.save(&sidecar) {
                tracing::error!("sidecar save failed for {}: {e}", sidecar.display());
            }
        }
    }

    fn schedule_suggestions(&mut self, on_demand: bool) {
        let Some(path) = self.primary_path() else {
            return;
        };
        if let Some(h) = self.suggest_pending.take() {
            h.cancel();
        }
        let edit = self.doc_mut(&path).edit.clone();
        let tx = self.tx.clone();
        let p = path.clone();
        let priority = if on_demand {
            Priority::Preview
        } else {
            Priority::Idle
        };
        self.suggest_pending = Some(self.scheduler.submit(priority, move || {
            let set = suggest::compute(&p, &edit);
            let _ = tx.send(Msg::Suggest(p, set));
        }));
    }

    fn queue_exports(&mut self) {
        let recipes = export_queue::default_recipes();
        let Some(recipe) = recipes.get(self.selected_recipe).cloned() else {
            return;
        };
        let paths: Vec<PathBuf> = self
            .session
            .selected
            .iter()
            .filter_map(|&i| self.session.entries.get(i).map(|e| e.path.clone()))
            .collect();
        for path in paths {
            let (edit, pipeline_version) = {
                let doc = self.doc_mut(&path);
                (doc.edit.clone(), doc.pipeline_version)
            };
            let index = self.exports.len();
            self.exports.push(ExportItem {
                source: path.clone(),
                recipe: recipe.name.clone(),
                status: ExportStatus::Pending,
            });
            let tx = self.tx.clone();
            let recipe = recipe.clone();
            self.scheduler.submit(Priority::Export, move || {
                let result = (|| -> Result<PathBuf, String> {
                    let decoded =
                        focale_core::decode::decode_file(&path).map_err(|e| e.to_string())?;
                    let input = focale_core::pipeline::RenderInput {
                        decoded: &decoded,
                        edit: &edit,
                        scale: 1.0,
                    };
                    let out = focale_core::pipeline::render(&input, pipeline_version)
                        .map_err(|e| e.to_string())?;
                    let bytes =
                        focale_export::encode(&out.image, &recipe).map_err(|e| e.to_string())?;
                    let out_path = export_queue::output_path(&path, &recipe.format);
                    if let Some(dir) = out_path.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(&out_path, bytes).map_err(|e| e.to_string())?;
                    Ok(out_path)
                })();
                let status = match result {
                    Ok(p) => ExportStatus::Done(p),
                    Err(e) => ExportStatus::Failed(e),
                };
                let _ = tx.send(Msg::Export(index, status));
            });
        }
    }

    fn handle_messages(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Base(path, Ok(base)) => {
                    self.decoding.remove(&path);
                    self.bases.insert(path.clone(), base.clone());
                    self.base_order.push(path.clone());
                    if self.base_order.len() > 8 {
                        let evict = self.base_order.remove(0);
                        if evict != path {
                            self.bases.remove(&evict);
                        }
                    }
                    if self.primary_path() == Some(path) {
                        self.spawn_render(base);
                    }
                }
                Msg::Base(path, Err(e)) => {
                    self.decoding.remove(&path);
                    tracing::error!("decode failed for {}: {e}", path.display());
                    self.warnings.clear();
                    self.frame = None;
                }
                Msg::Frame(path, Ok(f)) => {
                    if Some(&path) == self.primary_path().as_ref()
                        && f.version >= self.frame_uploaded
                    {
                        self.warnings = f.warnings.clone();
                        self.render_error = None;
                        self.upload_frame(frame, &f);
                        self.frame = Some(*f);
                        self.schedule_suggestions(false);
                    }
                }
                Msg::Frame(path, Err(e)) => {
                    if Some(&path) == self.primary_path().as_ref() {
                        tracing::error!("render failed for {}: {e}", path.display());
                        self.warnings.clear();
                        self.frame = None;
                        self.render_error = Some(e);
                    }
                }
                Msg::Thumb(path, image) => {
                    let handle = ctx.load_texture(
                        format!("thumb:{}", path.display()),
                        image,
                        Default::default(),
                    );
                    self.thumbs.insert(path, handle);
                }
                Msg::Export(index, status) => {
                    if let Some(item) = self.exports.get_mut(index) {
                        item.status = status;
                    }
                }
                Msg::AiMask(path, result) => {
                    self.segmenting = false;
                    if Some(&path) == self.primary_path().as_ref() {
                        match result {
                            Ok(mask) => {
                                self.push_mask(focale_core::masks::MaskShape::AiResolved(mask));
                            }
                            Err(e) => tracing::error!("segmentation failed: {e}"),
                        }
                    }
                }
                Msg::Suggest(path, set) => {
                    if Some(path) == self.primary_path() {
                        self.suggestions = set;
                    }
                }
            }
        }
    }

    fn upload_frame(&mut self, frame: &eframe::Frame, f: &PreviewFrame) {
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        let mut renderer = rs.renderer.write();
        if let Some(vp) = renderer.callback_resources.get_mut::<ViewportRenderer>() {
            vp.upload_image(
                &rs.device,
                &rs.queue,
                f.image.width(),
                f.image.height(),
                f.image.data(),
                f.version,
            );
            self.frame_uploaded = f.version;
        }
    }

    fn request_thumbnails(&mut self) {
        let paths: Vec<PathBuf> = self
            .session
            .entries
            .iter()
            .map(|e| e.path.clone())
            .filter(|p| !self.thumbs.contains_key(p) && !self.thumbs_requested.contains(p))
            .take(8)
            .collect();
        for path in paths {
            self.thumbs_requested.insert(path.clone());
            let tx = self.tx.clone();
            self.scheduler.submit(Priority::Thumbnail, move || {
                if let Ok(Some(jpeg)) = focale_core::decode::extract_thumbnail(&path)
                    && let Some(img) = thumbs::decode_thumbnail(&jpeg, 256)
                {
                    let _ = tx.send(Msg::Thumb(path, img));
                }
            });
        }
    }
}

impl eframe::App for FocaleApp {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.handle_messages(&ctx, frame);
        self.request_thumbnails();

        // ---- Top bar ----
        egui::Panel::top("top").show(root, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open directory…").clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    self.open_directory(dir);
                }
                if let Some(dir) = &self.session.dir {
                    ui.label(dir.display().to_string());
                }
                ui.separator();
                for (tool, label) in [
                    (Tool::Pan, "Pan"),
                    (Tool::Crop, "Crop"),
                    (Tool::LinearMask, "Linear"),
                    (Tool::RadialMask, "Radial"),
                    (Tool::BrushMask, "Brush"),
                    (Tool::Heal, "Heal"),
                    (Tool::Clone, "Clone"),
                    (Tool::AiObject, "AI object"),
                ] {
                    if ui.selectable_label(self.tool == tool, label).clicked() && self.tool != tool
                    {
                        self.tool = tool;
                        self.request_primary_preview();
                    }
                }
                if self.tool == Tool::BrushMask {
                    ui.add(
                        egui::Slider::new(&mut self.brush_radius, 0.005..=0.15).text("Brush size"),
                    );
                }
                ui.menu_button("AI mask", |ui| {
                    self.ai_mask_menu(ui);
                });
                if self.segmenting {
                    ui.spinner();
                }
                ui.separator();
                let zoom_label = match self.zoom {
                    None => "Fit".to_string(),
                    Some(z) => format!("{:.0}%", z * 100.0),
                };
                if ui.button(format!("Zoom: {zoom_label}")).clicked() {
                    self.zoom = match self.zoom {
                        None => Some(1.0),
                        Some(_) => None,
                    };
                    self.pan = egui::Vec2::ZERO;
                }
                ui.separator();
                egui::ComboBox::from_label("Rendering gamut")
                    .selected_text(self.gamut.display_name())
                    .show_ui(ui, |ui| {
                        for g in Gamut::ALL {
                            ui.selectable_value(&mut self.gamut, g, g.display_name());
                        }
                    });
            });
        });

        // ---- Status bar (docs/subsystems/app.md HARD: persistent keyed fields) ----
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Gamut: {}", self.gamut.display_name())).strong());
                ui.separator();
                let pv = self
                    .primary_path()
                    .and_then(|p| self.docs.get(&p))
                    .map(|d| d.pipeline_version)
                    .unwrap_or(focale_core::PIPELINE_VERSION);
                ui.label(format!("Pipeline: v{pv}"));
                // The one place a sidecar's pipeline version ever changes:
                // an explicit user upgrade to the current algorithms.
                if pv < focale_core::PIPELINE_VERSION
                    && ui
                        .small_button(format!("Upgrade to v{}", focale_core::PIPELINE_VERSION))
                        .clicked()
                    && let Some(path) = self.primary_path()
                {
                    self.doc_mut(&path).pipeline_version = focale_core::PIPELINE_VERSION;
                    self.dirty.insert(path, Instant::now());
                    self.request_primary_preview();
                }
                ui.separator();
                match self.cursor_probe {
                    Some((wk, disp)) => ui.label(format!(
                        "RGB {:.3} {:.3} {:.3} → {} {} {}",
                        wk[0], wk[1], wk[2], disp[0], disp[1], disp[2]
                    )),
                    None => ui.label("RGB — — —"),
                };
                ui.separator();
                let zoom_label = match self.zoom {
                    None => "Fit".to_string(),
                    Some(z) => format!("{:.0}%", z * 100.0),
                };
                ui.label(format!("Zoom: {zoom_label}"));
                ui.separator();
                if let Some(err) = &self.render_error {
                    ui.colored_label(ui.visuals().error_fg_color, format!("✘ {err}"));
                } else {
                    let warn = panels::warning_text(&self.warnings);
                    if warn.is_empty() {
                        ui.label("No warnings");
                    } else {
                        ui.colored_label(ui.visuals().warn_fg_color, format!("⚠ {warn}"));
                    }
                }
            });
        });

        // ---- Filmstrip ----
        egui::Panel::bottom("filmstrip")
            .default_size(120.0)
            .resizable(false)
            .show(root, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut clicked: Option<(usize, bool)> = None;
                        for (i, entry) in self.session.entries.iter().enumerate() {
                            let selected = self.session.selected.contains(&i);
                            let is_primary = self.session.primary == Some(i);
                            ui.vertical(|ui| {
                                let size = egui::vec2(96.0, 72.0);
                                let (rect, resp) =
                                    ui.allocate_exact_size(size, egui::Sense::click());
                                let stroke = if is_primary {
                                    egui::Stroke::new(2.0, Color32::WHITE)
                                } else if selected {
                                    egui::Stroke::new(2.0, Color32::GRAY)
                                } else {
                                    ui.visuals().widgets.noninteractive.bg_stroke
                                };
                                if let Some(tex) = self.thumbs.get(&entry.path) {
                                    egui::Image::new(tex).paint_at(ui, rect);
                                } else {
                                    ui.painter().rect_filled(
                                        rect,
                                        2.0,
                                        ui.visuals().extreme_bg_color,
                                    );
                                }
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    stroke,
                                    egui::StrokeKind::Inside,
                                );
                                if resp.clicked() {
                                    clicked = Some((i, ui.input(|s| s.modifiers.ctrl)));
                                }
                                let flag = match entry.live.flag {
                                    Flag::Pick => "⚑",
                                    Flag::Reject => "✕",
                                    Flag::None => "",
                                };
                                ui.label(format!(
                                    "{} {}{}",
                                    entry.file_name,
                                    "★".repeat(entry.live.rating as usize),
                                    flag
                                ));
                            });
                        }
                        if let Some((i, extend)) = clicked {
                            self.flush_dirty(true);
                            self.session.select(i, extend);
                            self.frame = None;
                            self.request_primary_preview();
                        }
                    });
                });
            });

        // ---- Right: stage panels + batch + export ----
        egui::Panel::right("panels")
            .default_size(320.0)
            .show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(path) = self.primary_path() else {
                        ui.label("Open a directory to begin.");
                        return;
                    };
                    // Rating / flags for the primary image.
                    if let Some(idx) = self.session.primary {
                        let mut live_changed = false;
                        {
                            let entry = &mut self.session.entries[idx];
                            live_changed |= panels::rating_widget(ui, &mut entry.live.rating);
                            ui.horizontal(|ui| {
                                for (flag, label) in [
                                    (Flag::Pick, "Pick"),
                                    (Flag::Reject, "Reject"),
                                    (Flag::None, "Clear"),
                                ] {
                                    if ui.small_button(label).clicked() {
                                        entry.live.flag = flag;
                                        live_changed = true;
                                    }
                                }
                            });
                        }
                        if live_changed {
                            self.dirty.insert(path.clone(), Instant::now());
                        }
                    }
                    ui.separator();
                    // Batch copy/paste (docs/subsystems/app.md).
                    ui.horizontal(|ui| {
                        if ui.button("Copy settings").clicked() {
                            self.clipboard = Some(self.doc_mut(&path).edit.clone());
                        }
                        let can_paste = self.clipboard.is_some();
                        if ui
                            .add_enabled(can_paste, egui::Button::new("Paste to selection"))
                            .clicked()
                            && let Some(edit) = self.clipboard.clone()
                        {
                            self.doc_mut(&path).edit = edit;
                            self.after_edit_change();
                        }
                    });
                    ui.separator();
                    // Stage panels in fixed order.
                    let warnings = self.warnings.clone();
                    let mut edit = self.doc_mut(&path).edit.clone();
                    if panels::stage_panels(ui, &mut edit, &warnings) {
                        self.doc_mut(&path).edit = edit;
                        self.after_edit_change();
                    }
                    ui.separator();
                    // AI suggestions (v1 stub, full affordance).
                    CollapsingSuggestions::show(ui, self);
                    ui.separator();
                    // Export.
                    ui.heading("Export");
                    let recipes = export_queue::default_recipes();
                    egui::ComboBox::from_label("Recipe")
                        .selected_text(recipes[self.selected_recipe].name.clone())
                        .show_ui(ui, |ui| {
                            for (i, r) in recipes.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_recipe, i, r.name.clone());
                            }
                        });
                    if ui.button("Export selection").clicked() {
                        self.queue_exports();
                    }
                    for item in self.exports.iter().rev().take(10) {
                        let name = item
                            .source
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        match &item.status {
                            ExportStatus::Pending => {
                                ui.label(format!("⏳ {name} ({})", item.recipe))
                            }
                            ExportStatus::Done(p) => {
                                ui.label(format!("✔ {name} → {}", p.display()))
                            }
                            ExportStatus::Failed(e) => ui.colored_label(
                                ui.visuals().error_fg_color,
                                format!("✘ {name}: {e}"),
                            ),
                        };
                    }
                });
            });

        // ---- Central viewport ----
        egui::CentralPanel::default().show(root, |ui| {
            let rect = ui.available_rect_before_wrap();
            let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            self.viewport_interaction(ui, rect, &response);
            let uv = self.uv_transform(rect);
            viewport::paint(
                ui,
                rect,
                ViewportCallback {
                    gamut: self.gamut,
                    uv_transform: uv,
                    background: 0.016,
                },
            );
        });

        self.keyboard(&ctx);
        self.flush_dirty(false);
        if !self.dirty.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn on_exit(&mut self) {
        self.flush_dirty(true);
    }
}

/// Suggestions section (kept out of `panels` because it needs app state).
struct CollapsingSuggestions;

impl CollapsingSuggestions {
    fn show(ui: &mut egui::Ui, app: &mut FocaleApp) {
        egui::CollapsingHeader::new("Suggestions")
            .default_open(false)
            .show(ui, |ui| {
                if app.suggestions.suggestions.is_empty() {
                    if app.suggestions.computed {
                        ui.label("No suggestions (model arrives in v2).");
                    } else {
                        ui.label("Computing when idle…");
                    }
                    if ui.small_button("Compute now").clicked() {
                        app.schedule_suggestions(true);
                    }
                    return;
                }
                let mut apply: Option<(usize, crate::suggest::Verdict)> = None;
                for (i, s) in app.suggestions.suggestions.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} → {:.2}", s.label, s.value));
                        if ui.small_button("Accept").clicked() {
                            apply = Some((i, crate::suggest::Verdict::Accept));
                        }
                        if ui.small_button("Tweak").clicked() {
                            apply = Some((i, crate::suggest::Verdict::Tweak));
                        }
                        if ui.small_button("Ignore").clicked() {
                            apply = Some((i, crate::suggest::Verdict::Ignore));
                        }
                    });
                }
                if let Some((i, verdict)) = apply {
                    let s = app.suggestions.suggestions[i].clone();
                    if verdict != crate::suggest::Verdict::Ignore
                        && let Some(path) = app.primary_path()
                    {
                        (s.apply)(&mut app.doc_mut(&path).edit, s.value);
                        app.after_edit_change();
                    }
                    app.suggestions.suggestions.remove(i);
                }
            });
    }
}

impl FocaleApp {
    /// Computes the uv transform (scale, offset) for the current zoom/pan.
    fn uv_transform(&self, rect: egui::Rect) -> [f32; 4] {
        let Some(f) = &self.frame else {
            return [1.0, 1.0, 0.0, 0.0];
        };
        let iw = f.image.width() as f32;
        let ih = f.image.height() as f32;
        let (rw, rh) = (rect.width(), rect.height());
        // Fit: image fully visible, centred.
        let fit = (rw / iw).min(rh / ih);
        let z = self.zoom.unwrap_or(fit);
        // Displayed image size in points: iw*z × ih*z. uv scale maps quad
        // uv (0..1 across rect) to image uv.
        let sx = rw / (iw * z);
        let sy = rh / (ih * z);
        let ox = 0.5 - sx * 0.5 + self.pan.x;
        let oy = 0.5 - sy * 0.5 + self.pan.y;
        [sx, sy, ox, oy]
    }

    /// Converts a screen position to normalized image coordinates.
    fn screen_to_image(&self, rect: egui::Rect, pos: egui::Pos2) -> Option<[f32; 2]> {
        let uv = self.uv_transform(rect);
        let qx = (pos.x - rect.left()) / rect.width();
        let qy = (pos.y - rect.top()) / rect.height();
        let ix = qx * uv[0] + uv[2];
        let iy = qy * uv[1] + uv[3];
        if (0.0..=1.0).contains(&ix) && (0.0..=1.0).contains(&iy) {
            Some([ix, iy])
        } else {
            None
        }
    }

    fn viewport_interaction(&mut self, ui: &egui::Ui, rect: egui::Rect, response: &egui::Response) {
        // Wheel zoom around centre; middle/secondary drag pans.
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if response.hovered() && scroll != 0.0 {
            let cur = self.zoom.unwrap_or_else(|| {
                self.frame
                    .as_ref()
                    .map(|f| {
                        (rect.width() / f.image.width() as f32)
                            .min(rect.height() / f.image.height() as f32)
                    })
                    .unwrap_or(1.0)
            });
            let next = (cur * (1.0 + scroll * 0.002)).clamp(0.02, 8.0);
            self.zoom = Some(next);
        }
        if response.dragged_by(egui::PointerButton::Middle)
            || (self.tool == Tool::Pan && response.dragged_by(egui::PointerButton::Primary))
        {
            let uv = self.uv_transform(rect);
            let d = response.drag_delta();
            self.pan -= egui::vec2(d.x / rect.width() * uv[0], d.y / rect.height() * uv[1]);
        }

        // Cursor colour probe (status bar).
        self.cursor_probe = None;
        if let (Some(pos), Some(f)) = (response.hover_pos(), &self.frame)
            && let Some([ix, iy]) = self.screen_to_image(rect, pos)
        {
            let x = ((ix * f.image.width() as f32) as u32).min(f.image.width() - 1);
            let y = ((iy * f.image.height() as f32) as u32).min(f.image.height() - 1);
            let wk = f.image.pixel(x, y);
            let mapped = map_to_gamut(wk, self.gamut);
            let disp = [
                (srgb_encode(mapped[0]) * 255.0 + 0.5) as u8,
                (srgb_encode(mapped[1]) * 255.0 + 0.5) as u8,
                (srgb_encode(mapped[2]) * 255.0 + 0.5) as u8,
            ];
            let _ = luminance_rec2020(wk);
            self.cursor_probe = Some((wk, disp));
        }

        // Tool drags in image space.
        let img_pos = response
            .interact_pointer_pos()
            .and_then(|p| self.screen_to_image(rect, p));
        match self.tool {
            Tool::Pan => {}
            Tool::Crop => self.crop_drag(response, img_pos),
            Tool::LinearMask | Tool::RadialMask => self.gradient_drag(response, img_pos),
            Tool::BrushMask => self.brush_drag(response, img_pos),
            Tool::Heal | Tool::Clone => self.retouch_drag(response, img_pos),
            Tool::AiObject => {
                if response.clicked()
                    && let Some(p) = img_pos
                {
                    let point = self.to_sensor_frame(p);
                    self.spawn_segmentation(SegmentRequest::Object(point));
                }
            }
        }
    }

    fn crop_drag(&mut self, response: &egui::Response, img_pos: Option<[f32; 2]>) {
        if response.drag_started() {
            self.drag_start = img_pos;
        }
        if response.drag_stopped()
            && let (Some(a), Some(b)) = (self.drag_start.take(), img_pos)
        {
            let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
            let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
            if x1 - x0 > 0.01
                && y1 - y0 > 0.01
                && let Some(path) = self.primary_path()
            {
                self.doc_mut(&path).edit.geometry.crop =
                    Some(focale_core::params::geometry::CropRect { x0, y0, x1, y1 });
                self.after_edit_change();
            }
        }
    }

    fn gradient_drag(&mut self, response: &egui::Response, img_pos: Option<[f32; 2]>) {
        if response.drag_started() {
            self.drag_start = img_pos;
        }
        if response.drag_stopped()
            && let (Some(a), Some(b)) = (self.drag_start.take(), img_pos)
        {
            use focale_core::masks::*;
            let shape = if self.tool == Tool::LinearMask {
                MaskShape::Linear(LinearGradientMask { start: a, end: b })
            } else {
                let rx = (b[0] - a[0]).abs().max(0.01);
                let ry = (b[1] - a[1]).abs().max(0.01);
                MaskShape::Radial(RadialGradientMask {
                    center: a,
                    radius: [rx, ry],
                    rotation: 0.0,
                    falloff: 0.5,
                })
            };
            self.push_mask(shape);
        }
    }

    fn brush_drag(&mut self, response: &egui::Response, img_pos: Option<[f32; 2]>) {
        if response.dragged()
            && let Some(p) = img_pos
            && self
                .brush_points
                .last()
                .map(|l| (l[0] - p[0]).abs() + (l[1] - p[1]).abs() > 0.002)
                .unwrap_or(true)
        {
            self.brush_points.push(p);
        }
        if response.drag_stopped() && !self.brush_points.is_empty() {
            use focale_core::masks::*;
            let stroke = BrushStroke {
                erase: false,
                radius: self.brush_radius,
                feather: 0.5,
                flow: 1.0,
                points: std::mem::take(&mut self.brush_points),
            };
            // Append to the last brush-mask adjustment if there is one.
            if let Some(path) = self.primary_path() {
                let doc = self.doc_mut(&path);
                let appended = doc
                    .edit
                    .local
                    .last_mut()
                    .and_then(|adj| adj.mask.components.last_mut())
                    .and_then(|c| match &mut c.shape {
                        MaskShape::Brush(b) => {
                            b.strokes.push(stroke.clone());
                            Some(())
                        }
                        _ => None,
                    })
                    .is_some();
                if appended {
                    self.after_edit_change();
                } else {
                    self.push_mask(MaskShape::Brush(BrushMask {
                        strokes: vec![stroke],
                    }));
                }
            }
        }
    }

    fn retouch_drag(&mut self, response: &egui::Response, img_pos: Option<[f32; 2]>) {
        let img_pos = img_pos.map(|p| self.to_sensor_frame(p));
        if response.drag_started() {
            self.drag_start = img_pos;
        }
        if response.dragged()
            && let Some(p) = img_pos
            && self.drag_start.is_some()
        {
            self.brush_points.push(p);
        }
        if response.drag_stopped()
            && let Some(start) = self.drag_start.take()
        {
            let dest = if self.brush_points.is_empty() {
                vec![start]
            } else {
                std::mem::take(&mut self.brush_points)
            };
            use focale_core::params::retouch::*;
            let stroke = RetouchStroke {
                mode: if self.tool == Tool::Heal {
                    RetouchMode::Heal
                } else {
                    RetouchMode::Clone
                },
                radius: self.brush_radius,
                feather: 0.5,
                opacity: 1.0,
                dest,
                // v1 default source: offset up-left by 2 radii; users
                // refine by dragging a new stroke.
                source_offset: [-self.brush_radius * 2.0, -self.brush_radius * 2.0],
            };
            if let Some(path) = self.primary_path() {
                self.doc_mut(&path).edit.retouch.strokes.push(stroke);
                self.after_edit_change();
            }
        }
    }

    /// Maps displayed-frame normalized coordinates to the sensor (pre-
    /// orientation) frame that masks and retouch strokes are stored in.
    /// Crop is exempt: the geometry stage applies it after orientation.
    fn to_sensor_frame(&self, p: [f32; 2]) -> [f32; 2] {
        let orientation = self
            .primary_path()
            .and_then(|path| {
                self.bases
                    .get(&path)
                    .map(|b| b.decoded.metadata.orientation)
            })
            .unwrap_or(1);
        let [x, y] = p;
        match orientation {
            2 => [1.0 - x, y],
            3 => [1.0 - x, 1.0 - y],
            4 => [x, 1.0 - y],
            5 => [y, x],
            6 => [y, 1.0 - x],
            7 => [1.0 - y, 1.0 - x],
            8 => [1.0 - y, x],
            _ => [x, y],
        }
    }

    fn ai_mask_menu(&mut self, ui: &mut egui::Ui) {
        use focale_core::masks::PersonPart;
        let available: bool = {
            let seg = self.segmenter.lock().unwrap();
            seg.available().iter().any(|(_, ok)| *ok)
        };
        if !available {
            ui.label("No models installed.");
            ui.label("Run scripts/fetch-models.sh to download them.");
            return;
        }
        if ui.button("Subject").clicked() {
            self.spawn_segmentation(SegmentRequest::Subject);
            ui.close();
        }
        if ui.button("Sky").clicked() {
            self.spawn_segmentation(SegmentRequest::Sky);
            ui.close();
        }
        if ui.button("Background").clicked() {
            self.spawn_segmentation(SegmentRequest::Background);
            ui.close();
        }
        if ui.button("Person").clicked() {
            self.spawn_segmentation(SegmentRequest::Person);
            ui.close();
        }
        ui.menu_button("Person part", |ui| {
            for (part, label) in [
                (PersonPart::FaceSkin, "Face skin"),
                (PersonPart::BodySkin, "Body skin"),
                (PersonPart::Hair, "Hair"),
                (PersonPart::Eyebrows, "Eyebrows"),
                (PersonPart::Sclera, "Eyes (sclera)"),
                (PersonPart::Iris, "Eyes (iris)"),
                (PersonPart::Lips, "Lips"),
                (PersonPart::Teeth, "Teeth"),
                (PersonPart::Clothing, "Clothing"),
            ] {
                if ui.button(label).clicked() {
                    self.spawn_segmentation(SegmentRequest::Part(part));
                    ui.close();
                }
            }
        });
    }

    /// Runs segmentation on the white-balanced working image of the preview
    /// base (sensor frame, so resolved bitmaps align with mask storage).
    fn spawn_segmentation(&mut self, request: SegmentRequest) {
        let Some(path) = self.primary_path() else {
            return;
        };
        let Some(base) = self.bases.get(&path).cloned() else {
            return;
        };
        let wb = self.doc_mut(&path).edit.white_balance.clone();
        let segmenter = self.segmenter.clone();
        let tx = self.tx.clone();
        self.segmenting = true;
        self.scheduler.submit(Priority::Preview, move || {
            let mut image = focale_core::image::ImageRgbF32::from_data(
                base.decoded.width,
                base.decoded.height,
                base.decoded.pixels.clone(),
            );
            focale_core::pipeline::v1::white_balance::apply(
                &mut image,
                &wb,
                &base.decoded.metadata,
            );
            let result = {
                let mut seg = segmenter.lock().unwrap();
                match request {
                    SegmentRequest::Subject => seg.subject(&image),
                    SegmentRequest::Sky => seg.sky(&image),
                    SegmentRequest::Background => seg.background(&image),
                    SegmentRequest::Person => seg.person(&image),
                    SegmentRequest::Part(part) => seg.person_part(&image, part),
                    SegmentRequest::Object(point) => seg.object_at(&image, point),
                }
            }
            .map_err(|e| e.to_string());
            let _ = tx.send(Msg::AiMask(base.path.clone(), result));
        });
    }

    fn push_mask(&mut self, shape: focale_core::masks::MaskShape) {
        use focale_core::masks::*;
        use focale_core::params::local::{LocalAdjustment, LocalParams};
        let Some(path) = self.primary_path() else {
            return;
        };
        let doc = self.doc_mut(&path);
        let n = doc.edit.local.len() + 1;
        doc.edit.local.push(LocalAdjustment {
            enabled: true,
            mask: MaskGroup {
                name: format!("Mask {n}"),
                components: vec![MaskComponent {
                    op: MaskOp::Add,
                    invert: false,
                    feather: 0.0,
                    density: 1.0,
                    shape,
                }],
            },
            adjustments: LocalParams::default(),
        });
        self.after_edit_change();
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.session.primary else {
            return;
        };
        let mut live_changed = false;
        ctx.input(|i| {
            let entry = &mut self.session.entries[idx];
            for (key, rating) in [
                (Key::Num0, 0u8),
                (Key::Num1, 1),
                (Key::Num2, 2),
                (Key::Num3, 3),
                (Key::Num4, 4),
                (Key::Num5, 5),
            ] {
                if i.key_pressed(key) {
                    entry.live.rating = rating;
                    live_changed = true;
                }
            }
            if i.key_pressed(Key::P) {
                entry.live.flag = Flag::Pick;
                live_changed = true;
            }
            if i.key_pressed(Key::X) {
                entry.live.flag = Flag::Reject;
                live_changed = true;
            }
            if i.key_pressed(Key::U) {
                entry.live.flag = Flag::None;
                live_changed = true;
            }
        });
        if live_changed && let Some(path) = self.primary_path() {
            self.dirty.insert(path, Instant::now());
        }
        // Arrow keys move the primary selection.
        let (mut left, mut right) = (false, false);
        ctx.input(|i| {
            left = i.key_pressed(Key::ArrowLeft);
            right = i.key_pressed(Key::ArrowRight);
        });
        if left || right {
            let len = self.session.entries.len();
            if len > 0 {
                let cur = self.session.primary.unwrap_or(0);
                let next = if right {
                    (cur + 1).min(len - 1)
                } else {
                    cur.saturating_sub(1)
                };
                if next != cur {
                    self.flush_dirty(true);
                    self.session.select(next, false);
                    self.frame = None;
                    self.request_primary_preview();
                }
            }
        }
    }
}

/// A queued segmentation request.
#[derive(Debug, Clone, Copy)]
enum SegmentRequest {
    Subject,
    Sky,
    Background,
    Person,
    Part(focale_core::masks::PersonPart),
    Object([f32; 2]),
}
