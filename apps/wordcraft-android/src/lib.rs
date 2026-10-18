//! Minimal Android shell for WordCraft.
//!
//! This deliberately supplies no desktop-only services: native file pickers,
//! printing, clipboard bridges, control server and platform integrations remain
//! desktop work. The first Android milestone is a safe, installable editor shell.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "android")]
use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{Global, JByteArray, JObject, JValue},
};
#[cfg(target_os = "android")]
use std::{fs, io::ErrorKind, path::PathBuf, sync::Arc};
#[cfg(target_os = "android")]
use wordcraft_engine::Session;
#[cfg(target_os = "android")]
use wordcraft_ui_egui::{Inbox, Services, WordApp};

#[cfg(target_os = "android")]
const PICKED_URI_FILE: &str = "wordcraft-saf-picked-uri";

/// Android's SAF bridge. Java owns the URI grant and reads it through
/// `ContentResolver`; Rust receives ordinary owned bytes through the existing
/// asynchronous inbox used by the web host.
#[cfg(target_os = "android")]
struct AndroidSaf {
    app: winit::platform::android::activity::AndroidApp,
    picked_uri: PathBuf,
}

#[cfg(target_os = "android")]
impl AndroidSaf {
    fn new(app: winit::platform::android::activity::AndroidApp) -> Result<Self, String> {
        let data_dir = app.internal_data_path().ok_or_else(|| "Android did not provide an application data directory".to_owned())?;
        let picked_uri = data_dir.join(PICKED_URI_FILE);
        if let Err(error) = fs::remove_file(&picked_uri) {
            if error.kind() != ErrorKind::NotFound {
                log::warn!("could not clear an old SAF selection: {error}");
            }
        }
        Ok(Self { app, picked_uri })
    }

    fn request_open(&self) {
        if let Err(error) = fs::remove_file(&self.picked_uri) {
            if error.kind() != ErrorKind::NotFound {
                log::warn!("could not clear an old SAF selection: {error}");
            }
        }
        let app = self.app.clone();
        // The picker must start on Android's Java main thread. `AndroidApp`
        // keeps the Activity's global JNI reference alive until this callback runs.
        self.app.run_on_java_main_thread(Box::new(move || {
            if let Err(error) = start_open_document(&app) {
                log::error!("could not show Android document picker: {error}");
            }
        }));
    }

    fn take_picked_file(&self) -> Result<Option<(String, Vec<u8>)>, String> {
        let uri = match fs::read_to_string(&self.picked_uri) {
            Ok(uri) => uri,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("could not receive selected file: {error}")),
        };
        if let Err(error) = fs::remove_file(&self.picked_uri) {
            log::warn!("could not consume SAF selection: {error}");
        }
        let uri = uri.trim();
        if uri.is_empty() {
            return Err("the Android document picker returned an empty URI".to_owned());
        }
        let bytes = read_open_document(&self.app, uri)?;
        // Document providers conventionally retain the source filename at the end of the URI.
        // Keeping its suffix lets the engine select the correct importer.
        let name = uri.rsplit(['/', ':']).next().filter(|name| !name.is_empty()).unwrap_or("selected-document").to_owned();
        Ok(Some((name, bytes)))
    }
}

/// The `android-activity` API intentionally exposes the VM and Activity as raw
/// JNI handles. The two conversions below follow that crate's documented
/// lifetime contract: `app` is retained for the entire call and its Activity
/// handle is an unowned global reference. Every Java call attaches its thread
/// and turns Java/JNI failures into a normal error.
#[cfg(target_os = "android")]
fn with_activity<T>(
    app: &winit::platform::android::activity::AndroidApp,
    f: impl FnOnce(&mut jni::Env<'_>, &JObject<'_>) -> jni::errors::Result<T>,
) -> Result<T, String> {
    // SAFETY: AndroidApp documents `vm_as_ptr` as a valid JavaVM pointer for
    // precisely this conversion, while this AndroidApp remains alive.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| {
        let activity = app.activity_as_ptr().cast();
        // SAFETY: AndroidApp documents this as an unowned global Activity ref
        // valid until `app` is dropped; the reference is used only in this call.
        let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&activity) }?;
        f(env, activity.as_ref())
    })
    .map_err(|error: jni::errors::Error| format!("Android Java call failed: {error}"))
}

#[cfg(target_os = "android")]
fn start_open_document(app: &winit::platform::android::activity::AndroidApp) -> Result<(), String> {
    with_activity(app, |env, activity| {
        env.call_method(activity, jni_str!("startOpenDocument"), jni_sig!("()V"), &[])?;
        Ok(())
    })
}

#[cfg(target_os = "android")]
fn read_open_document(app: &winit::platform::android::activity::AndroidApp, uri: &str) -> Result<Vec<u8>, String> {
    with_activity(app, |env, activity| {
        let uri = env.new_string(uri)?;
        let value = env.call_method(activity, jni_str!("readOpenDocument"), jni_sig!("(Ljava/lang/String;)[B"), &[JValue::Object(&uri)])?;
        let bytes = JByteArray::cast_local(env, value.l()?)?;
        env.convert_byte_array(bytes)
    })
}

/// The eframe-facing layer stays here rather than in `wordcraft-ui-egui`: the
/// shared UI deliberately has no dependency on any windowing framework.
#[cfg(target_os = "android")]
struct AndroidShell {
    app: WordApp,
    saf: Option<Arc<AndroidSaf>>,
    /// Direct handle to the activity app: winit 0.30's Android backend drops
    /// `ContentRectChanged` (a `TODO!` in its source), so the shell reads
    /// `AndroidApp::content_rect()` itself to keep egui clear of the soft keyboard.
    activity_app: winit::platform::android::activity::AndroidApp,
    inbox: Inbox,
}

#[cfg(target_os = "android")]
impl AndroidShell {
    /// Soft keyboards overlap the surface: Android's `adjustResize` reports the visible
    /// region through `AndroidApp::content_rect()`, which winit never forwards to egui.
    /// Shrinking the frame's `screen_rect` to that region lifts every egui surface —
    /// bottom bars, sheets and dialogs — above the keyboard without per-widget work.
    /// The raw pointer pixels of `content_rect` are physical pixels; `pixels_per_point`
    /// converts them into the points egui lays out in.
    fn lift_above_soft_input(&self, raw: &mut egui::RawInput) {
        let Some(mut rect) = raw.screen_rect else { return };
        let content = self.activity_app.content_rect();
        let ppp = raw.pixels_per_point.unwrap_or(1.0).max(f32::EPSILON);
        let visible = egui::Rect::from_min_size(
            egui::pos2(content.left as f32 / ppp, content.top as f32 / ppp),
            egui::vec2(content.width() as f32 / ppp, content.height() as f32 / ppp),
        );
        // Only shrink (never grow): the fallback inset values already reserve space
        // for the system bars, and an equal rect means the keyboard is hidden.
        if visible.width() > 1.0 && visible.height() > 1.0 && visible.height() < rect.height() {
            rect.min = rect.min.max(visible.min);
            rect.max = rect.max.min(visible.max);
            raw.screen_rect = Some(rect);
        }
    }
}

#[cfg(target_os = "android")]
impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(saf) = &self.saf {
            match saf.take_picked_file() {
                Ok(Some(file)) => match self.inbox.lock() {
                    Ok(mut inbox) => inbox.push(file),
                    Err(error) => self.app.status(format!("Android file inbox is unavailable: {error}")),
                },
                Ok(None) => {}
                Err(error) => self.app.status(error),
            }
        }
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.lift_above_soft_input(raw);
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

/// Android's NativeActivity calls this exported symbol on its UI thread.
///
/// `AndroidApp` comes from winit's re-export, so its android-activity version
/// always matches the version eframe/winit require.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    if let Err(error) = run(app) {
        // NativeActivity owns process lifetime. Returning lets Android close this
        // activity without a panic or an unsafe process exit.
        log::error!("WordCraft Android could not start: {error}");
    }
}

#[cfg(target_os = "android")]
fn run(app: winit::platform::android::activity::AndroidApp) -> eframe::Result {
    let region = wordcraft_ui_egui::i18n::system_region();
    let mut session = Session::new(wordcraft_engine::blank_document(&region));
    session.system_region = region;
    // Touch screens do not have room for the desktop ruler. Keep the global Session default for
    // desktop intact: View › Ruler remains available in the mobile command sheet.
    session.view.ruler = false;
    // A phone is narrower than a page at 100%: start at Page Width, as Word's phone layout does,
    // so the first page fills the screen instead of running off both edges.
    session.view.fit = "pageWidth".into();

    let inbox: Inbox = Default::default();
    let saf = AndroidSaf::new(app.clone()).map(Arc::new);
    let mut services = Services { inbox: Some(inbox.clone()), ..Default::default() };
    if let Ok(bridge) = &saf {
        let bridge = bridge.clone();
        services.open_async = Some(Box::new(move |_| bridge.request_open()));
    } else if let Err(error) = &saf {
        log::error!("Android SAF is unavailable: {error}");
    }
    // A second handle for the shell's soft-input lift (see `AndroidShell`).
    let activity_app = app.clone();

    let options = eframe::NativeOptions {
        android_app: Some(app),
        viewport: egui::ViewportBuilder::default().with_title("WordCraft").with_app_id("ai.storyteller.wordcraft.android"),
        ..Default::default()
    };

    let mut app = AndroidShell { app: WordApp::new(session, services), saf: saf.ok(), activity_app, inbox };
    // Mobile launches on the home screen (New / Open), as Word and the desktop shell do when
    // nothing was handed over at launch: `has_document()` stays false until the user creates
    // or opens a document.
    app.app.start_screen();
    eframe::run_native("WordCraft", options, Box::new(move |_| Ok(Box::new(app))))
}
