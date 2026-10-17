//! Minimal Android shell for WordCraft.
//!
//! This deliberately supplies no desktop-only services: native file pickers,
//! printing, clipboard bridges, control server and platform integrations remain
//! desktop work. The first Android milestone is a safe, installable editor shell.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "android")]
use wordcraft_engine::Session;
#[cfg(target_os = "android")]
use wordcraft_ui_egui::{Services, WordApp};

/// The eframe-facing layer stays here rather than in `wordcraft-ui-egui`: the
/// shared UI deliberately has no dependency on any windowing framework.
#[cfg(target_os = "android")]
struct AndroidShell(WordApp);

#[cfg(target_os = "android")]
impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.0.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.0.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
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

    let options = eframe::NativeOptions {
        android_app: Some(app),
        viewport: egui::ViewportBuilder::default()
            .with_title("WordCraft")
            .with_app_id("ai.storyteller.wordcraft.android"),
        ..Default::default()
    };

    eframe::run_native(
        "WordCraft",
        options,
        Box::new(move |_| Ok(Box::new(AndroidShell(WordApp::new(session, Services::default()))))),
    )
}
