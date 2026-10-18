//! Android's touch-first command shell.
//!
//! This deliberately contains presentation only. Every actionable item dispatches the same
//! command id as the desktop ribbon through [`WordApp::run`].

use egui::{Align, Align2, Area, Button, Frame, Id, Layout, Order, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::theme::{Tokens, regular, semibold};
use crate::{WordApp, icons};

const SHEET: &str = "wordcraft.mobile.sheet";
/// Android's status bar is normally 24dp in portrait. `eframe` 0.36 passes the
/// `AndroidApp` to winit but does not expose its changing WindowInsets to the
/// egui app, so this conservative value keeps touch chrome below the system UI.
/// Shared with the file browser's top bar, which needs the same clearance; it is
/// still not a desktop safe-area policy.
pub(crate) const STATUS_INSET: f32 = 24.0;
/// Navigation insets are likewise not available through eframe's Android path.
/// The gesture-indicator strip needs breathing room even at zero inset, so the
/// tab bar reserves it and draws its labels clear of the screen edge.
pub(crate) const NAV_INSET: f32 = 16.0;
const CORE_TABS: &[&str] = &["Home", "Insert", "Layout", "Review", "View"];
const MOBILE_RECENT_LIMIT: usize = 8;

// ── Design tokens ────────────────────────────────────────────────────────────────────────────
// Every mobile surface — the shell's bars, the command sheets, the bottom-sheet dialogs and the
// file browser page — reads these, so a measurement lands everywhere at once instead of each
// screen drifting its own way. All of them sit on an 8dp grid; colours are never restated here,
// they come from [`Tokens`] (accent, text, text_dim, canvas/panel, border), so the tokens track
// the theme.
/// The horizontal safe margin the top bars, the sheets, the browser page and lists all share.
pub(crate) const EDGE: f32 = 16.0;
/// The top bar's and a sheet's title row height (was 64 on the bars, 83 in the dialogs).
pub(crate) const BAR_H: f32 = 56.0;
/// The height of a dialog's filled or outlined action button.
pub(crate) const BUTTON_H: f32 = 40.0;
/// The smallest tappable control.
pub(crate) const TOUCH: f32 = 48.0;
/// A top-bar icon button's side.
pub(crate) const ICON_BTN: f32 = 48.0;
/// The one corner radius for controls: buttons, fields, list rows, sheet actions. Surfaces keep
/// MD3's own shape (a bottom sheet's 28dp top corners); everything that is a control is 12dp.
pub(crate) const RADIUS: f32 = 12.0;
/// A file list's row height (MD3's single-line list row; was 97).
pub(crate) const LIST_ROW: f32 = 56.0;

/// The one selection treatment: a 12% accent wash under accent content — the tab bar's pill and a
/// sheet's selected tab. MD3's secondary-container role, mixed from the single accent so the two
/// can never drift apart again (they used to be a pale wash and a saturated fill).
pub(crate) fn selection(t: &Tokens) -> egui::Color32 {
    t.accent.linear_multiply(0.12)
}

/// Runs `add` with egui's selection colour set to the shared treatment, so a selected selectable
/// label (the Find sheet's disclosure rows) reads like the pills instead of egui's own selection
/// fill. Document text selection keeps the theme's own `Tokens::selection`.
pub(crate) fn selection_scope<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.scope(|ui| {
        ui.style_mut().visuals.selection.bg_fill = selection(&t);
        ui.style_mut().visuals.selection.stroke = Stroke::new(1.0, t.accent);
        add(ui)
    })
    .inner
}

/// A frameless icon button: no container at rest, a hover/press state layer in its [`RADIUS`]
/// corner, an [`ICON_BTN`] touch target. The bars' shared control, so the top bar, the browser
/// page's bar and the sheets' title rows speak one language. Always senses clicks — a disabled
/// button paints dim and the caller says so in the status line instead of going mute.
pub(crate) fn icon_button(ui: &mut Ui, icon: &str, tip: &str, enabled: bool, t: &Tokens) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ICON_BTN, ICON_BTN), Sense::click());
    if enabled {
        let state = crate::widgets::drawn_state(ui, &response, false);
        ui.painter().rect_filled(rect, RADIUS, state.fill);
    }
    let color = if enabled { t.text } else { t.text_dim };
    // The accent slot rides along for the few glyphs with a second colour; the bars' icons are
    // monochrome, and the home-folder glyph loses its accent door with the same call.
    icons::paint(ui.painter(), rect.shrink(12.0), icon, color, color);
    response.on_hover_text(tip).clicked()
}

/// A sheet action button: `primary` is the accent-filled main action, anything else is outlined.
/// `width` is the button's own width; the row computes it. True when clicked.
pub(crate) fn sheet_button(ui: &mut Ui, label: &str, primary: bool, width: f32, enabled: bool, t: &Tokens) -> bool {
    let text = egui::RichText::new(label).font(regular(13.0));
    let button = if primary {
        egui::Button::new(text.color(if enabled { t.on_accent } else { t.text_dim })).fill(if enabled { t.accent } else { t.border })
    } else {
        let stroke = if enabled { t.border_strong } else { t.border };
        egui::Button::new(text.color(if enabled { t.text } else { t.text_dim })).fill(t.ribbon).stroke(Stroke::new(1.0, stroke))
    };
    ui.add_enabled(enabled, button.min_size(vec2(width, BUTTON_H)).corner_radius(egui::CornerRadius::same(RADIUS as u8))).clicked()
}

/// Restates the one control radius on the widget visuals a text field takes its corner from
/// (egui has no `TextEdit` corner builder) — the same restatement the browser's name field does.
pub(crate) fn field_radius(ui: &mut Ui) {
    let radius = egui::CornerRadius::same(RADIUS as u8);
    let visuals = ui.visuals_mut();
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
}

/// A sheet's title row: the title left-aligned at [`EDGE`] (MD3's bottom-sheet title), a 48dp
/// close at the right, the whole row [`BAR_H`] tall. Replaces egui's window title bar on the
/// phone, whose separator and centring were desktop chrome. True when close is clicked.
pub(crate) fn sheet_title(ui: &mut Ui, title: &str, t: &Tokens) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.set_height(BAR_H);
        // Painter text, not a `Label`: a `Window`'s content ui centres its children
        // (`add_sized` lays out with `centered_and_justified`), so the title sat centred no
        // matter the label's own `halign` — MD3's bottom-sheet title is left-aligned.
        let spacing = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - TOUCH - spacing).max(TOUCH);
        let (rect, _) = ui.allocate_exact_size(vec2(width, BAR_H), Sense::hover());
        ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, title, semibold(16.0), t.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            close = icon_button(ui, "close", tl!("Close"), true, t);
        });
    });
    close
}

/// A row of selection pills — a sheet's tabs. The selected one is the shared treatment
/// ([`selection`] wash, accent text), the rest plain text, 8dp apart. Returns the index of a
/// pill that was tapped.
pub(crate) fn pills(ui: &mut Ui, selected: usize, labels: &[&str]) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (index, label) in labels.iter().enumerate() {
            let on = index == selected;
            let text = egui::RichText::new(tl!(*label)).font(if on { semibold(13.0) } else { regular(13.0) }).color(if on { t.accent } else { t.text });
            let button = egui::Button::new(text)
                .fill(if on { selection(&t) } else { egui::Color32::TRANSPARENT })
                .stroke(Stroke::NONE)
                .corner_radius(egui::CornerRadius::same(RADIUS as u8))
                .min_size(vec2(0.0, 36.0));
            if ui.add(button).clicked() {
                picked = Some(index);
            }
        }
    });
    picked
}

/// The Find dialog's action row on the phone: one line of equal-width [`BUTTON_H`] buttons on the
/// 8dp grid — the main action filled, the rest outlined — where the desktop's four same-weight
/// buttons wrapped and hung off the sheet. `Enter` still finds next and `Escape` still closes.
/// Returns whether the dialog should close.
pub(crate) fn find_buttons(ui: &mut Ui, app: &mut WordApp, replace_mode: bool, opts: &serde_json::Value, message: &mut String) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut find_next = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        // Four 14sp labels with the global 8dp padding overflow a 360dp phone's sheet, so the row
        // uses its own tighter padding and lets the width follow the space it actually has.
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        let mut row: Vec<(&str, bool)> = Vec::new();
        if replace_mode {
            row.push(("Replace All", false));
            row.push(("Replace", false));
        }
        row.push(("Find Next", true));
        row.push(("Close", false));
        let width = ((ui.available_width() - 8.0 * (row.len() as f32 - 1.0)) / row.len() as f32).max(56.0);
        for (label, primary) in row {
            if sheet_button(ui, tl!(label), primary, width, true, &t) {
                match label {
                    "Replace All" => match app.run("edit.replaceAll", opts.clone()) {
                        Ok(r) => *message = format!("All done. We made {} replacements.", r["replaced"]),
                        Err(e) => *message = e,
                    },
                    "Replace" => {
                        let _ = app.run("edit.replace", opts.clone());
                    }
                    "Find Next" => find_next = true,
                    _ => close = true,
                }
            }
        }
    });
    if find_next {
        // One step per click: `edit.find` would select a match itself, and the step after it skip one.
        match app.run("edit.findNext", opts.clone()) {
            Ok(r) => *message = format!("Match {} of {}", r["index"].as_u64().unwrap_or(0) + 1, r["count"]),
            Err(e) => *message = e,
        }
    }
    close
}

/// One original glyph per navigation tab, so the bar is scannable at a glance
/// (MD3: Filled-style 24dp navigation icons). Drawn in `icons.rs` style.
fn tab_icon(tab: &str) -> &'static str {
    match tab {
        "Home" => "tabHome",
        "Insert" => "tabInsert",
        "Layout" => "tabLayout",
        "Review" => "tabReview",
        "View" => "tabView",
        _ => "tab",
    }
}

/// A small, deliberately curated touch surface over the existing command registry. The command
/// ids are the source of truth; unavailable ids are simply omitted at paint time so a build with
/// a smaller registry remains usable.
const COMMANDS: &[(&str, &[(&str, &[&str])])] = &[
    (
        "Home",
        &[
            ("Font", &["format.bold", "format.italic", "format.underline", "format.color"]),
            ("Paragraph", &["para.bullets", "para.numbering", "para.alignLeft", "para.alignCenter"]),
            ("Clipboard", &["edit.paste", "edit.copy", "edit.cut"]),
            ("Draw", &["draw.select", "draw.lasso", "draw.eraser", "draw.pen", "draw.pencil", "draw.highlighter"]),
        ],
    ),
    (
        "Insert",
        &[
            ("Pages & tables", &["insert.table", "table.fromText"]),
            ("Illustrations", &["insert.picture", "insert.shape", "insert.icon"]),
            ("Text", &["insert.textBox", "insert.link", "insert.equation"]),
        ],
    ),
    (
        "Layout",
        &[
            ("Page setup", &["layout.margins", "layout.orientation", "layout.size", "layout.columns"]),
            ("Page", &["layout.break", "layout.hyphenation", "layout.pageSetup"]),
        ],
    ),
    (
        "Review",
        &[
            ("Comments", &["review.newComment", "review.comments"]),
            ("Tracking", &["review.trackChanges", "review.accept", "review.reject"]),
            ("Proofing", &["review.spelling", "review.wordCount"]),
        ],
    ),
    (
        "View",
        &[
            ("Document", &["view.printLayout", "view.navigationPane", "view.immersive"]),
            ("Movement", &["view.vertical", "view.sideToSide", "view.fieldCodes"]),
        ],
    ),
    ("Table Design", &[("Table styles", &["table.style", "table.shading", "table.borders"])]),
    (
        "Table Layout",
        &[
            ("Rows & columns", &["table.insertRowAbove", "table.insertRowBelow", "table.insertColumnLeft", "table.insertColumnRight"]),
            ("Table", &["table.merge", "table.split", "table.deleteTable", "table.properties"]),
        ],
    ),
    ("Picture Format", &[("Picture", &["picture.change", "picture.altText", "picture.crop", "picture.wrap"])]),
    ("Shape Format", &[("Shape", &["shape.fill", "shape.outline", "shape.wrap", "shape.altText"])]),
    ("Equation", &[("Equation", &["equation.linear", "equation.professional", "equation.insert"])]),
    ("Chart Design", &[("Chart", &["chart.changeType", "chart.editData", "chart.addElement"])]),
    ("SmartArt Design", &[("SmartArt", &["smartArt.addShape", "smartArt.changeLayout", "smartArt.changeColors"])]),
];

fn sheet_tab(ui: &Ui) -> Option<String> {
    ui.data(|d| d.get_temp(Id::new(SHEET)))
}

fn set_sheet(ui: &Ui, tab: Option<&str>) {
    ui.data_mut(|d| match tab {
        Some(tab) => {
            d.insert_temp(Id::new(SHEET), tab.to_owned());
        }
        None => {
            d.remove::<String>(Id::new(SHEET));
        }
    });
}

fn available_tabs(_app: &WordApp) -> Vec<String> {
    CORE_TABS.iter().map(|tab| (*tab).to_owned()).collect()
}

/// A top-bar icon button: the shared frameless 48dp control, dim when `enabled` says the command
/// would do nothing (a tap then explains itself in the status line instead of looking alive).
fn top_button(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, id: &str, params: serde_json::Value, enabled: bool, t: &Tokens) {
    if icon_button(ui, icon, label, enabled, t) {
        if enabled {
            run(app, id, params);
        } else if id == "edit.undo" || id == "edit.redo" {
            // The registry runs these as a silent no-op without history; the tap still deserves an
            // answer (the review's "blank document, lit undo").
            app.status(tl!(if id == "edit.undo" { "Nothing to undo" } else { "Nothing to redo" }));
        }
    }
}

/// The mobile shell's own glyph picks. Where the desktop ribbon keeps its lettered tiles
/// (`bold` = "B", `fontcolor` = "A" over a colour bar), the touch grid draws the geometric marks
/// instead — letters read cheap at a 24dp touch icon. New names, so the desktop keeps drawing
/// exactly what it has always drawn; everything else falls through to the shared [`command_icon`].
fn command_glyph(id: &str) -> &str {
    match id {
        "format.bold" => "boldGeo",
        "format.color" => "fontcolorDrop",
        "format.italic" => "italicGeo",
        "format.underline" => "underlineGeo",
        _ => crate::widgets::command_icon(id),
    }
}

/// Whether a command is on at the selection — the active tile's treatment. The reads are the
/// desktop ribbon's own (`format.state`, the resolved paragraph, [`wordcraft_engine::cmd::para::list_buttons`]);
/// commands without a toggle state (insertions, dialogs) are simply never active.
fn command_active(id: &str, st: &serde_json::Value, align: wordcraft_doc::Align, lists: wordcraft_engine::cmd::para::ListButtons) -> bool {
    let flag = |key: &str| st.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false);
    match id {
        "format.bold" => flag("bold"),
        "format.color" => false,
        "format.italic" => flag("italic"),
        "format.underline" => flag("underline"),
        "para.alignCenter" => align == wordcraft_doc::Align::Center,
        "para.alignLeft" => align == wordcraft_doc::Align::Left,
        "para.bullets" => lists.bullets,
        "para.numbering" => lists.numbering,
        _ => false,
    }
}

/// One line of label text that never exceeds `max`: the longest char-boundary prefix that fits,
/// with an ellipsis. Measured with the same layout the painter uses, so the cut is exact — the
/// old clip left "rt Text to Table" mid-word hanging out of a tile.
fn elided(ui: &Ui, text: &str, font: egui::FontId, max: f32) -> String {
    let width = |s: &str| {
        let font = font.clone();
        ui.ctx().fonts_mut(|f| f.layout_no_wrap(s.to_owned(), font, egui::Color32::WHITE).size().x)
    };
    if width(text) <= max {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().take(128).collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        let candidate: String = chars[..mid].iter().chain(std::iter::once(&'…')).collect();
        if width(&candidate) <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().chain(std::iter::once(&'…')).collect()
}

/// One command tile: a single grid item ([`TOUCH`]-tall, `tile_w` wide so the column has no
/// trailing blank), hover/press feedback like [`icon_button`], the command's glyph, and a label
/// elided to the tile edge. On commands that report a state the active tile takes the shared
/// selection treatment and an accent glyph. True when the command ran (the sheet closes).
fn command_button(ui: &mut Ui, app: &mut WordApp, id: &str, active: bool, tile_w: f32, label_sp: f32, t: Tokens) -> bool {
    let Some(spec) = app.session.registry.get(id) else { return false };
    let label = tl!(spec.label).to_owned();
    let enabled = crate::widgets::enabled(app, id);
    let on = active && enabled;
    let (rect, response) = ui.allocate_exact_size(vec2(tile_w.max(TOUCH), 64.0), Sense::click());
    let state = crate::widgets::drawn_state(ui, &response, false);
    let fill = if on { selection(&t) } else { state.fill };
    if fill != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, RADIUS, fill);
    }
    let color = if enabled { t.text } else { t.text_dim };
    let icon_rect = egui::Rect::from_center_size(rect.center_top() + vec2(0.0, 20.0), vec2(24.0, 24.0));
    icons::paint(ui.painter(), icon_rect, command_glyph(id), if on { t.accent } else { color }, t.accent);
    let font = regular(label_sp);
    let shown = elided(ui, &label, font.clone(), rect.width() - 8.0);
    ui.painter().text(rect.center_bottom() - vec2(0.0, 11.0), Align2::CENTER_CENTER, shown, font, color);
    let _ = response.clone().on_hover_text(&label);
    // A disabled tile paints dim and stays inert, as `add_enabled` did before it.
    if enabled && response.clicked() {
        if let Err(error) = app.run(id, json!({})) {
            app.status(error);
        }
        return true;
    }
    false
}

fn command_sheet(ui: &mut Ui, app: &mut WordApp, tab: &str, t: Tokens) {
    let Some((_, groups)) = COMMANDS.iter().find(|(name, _)| *name == tab) else { return };
    let height = (ui.ctx().content_rect().height() * 0.52).clamp(220.0, 480.0);
    let ctx = ui.ctx().clone();
    let progress = ctx.animate_bool_with_time(Id::new("wordcraft.mobile.sheet.animation"), true, 0.300);
    // The foreground overlay leaves the system chrome alone and closes the sheet
    // before a canvas tap can reach the document underneath.
    let canvas = ctx.content_rect();
    Area::new(Id::new("wordcraft.mobile.sheet.scrim")).order(Order::Foreground).fixed_pos(canvas.min).show(&ctx, |ui| {
        let (rect, response) = ui.allocate_exact_size(canvas.size(), Sense::click());
        ui.painter().rect_filled(rect, 0.0, t.text.linear_multiply(0.06));
        if response.clicked() {
            set_sheet(ui, None);
        }
    });
    // The toggles' state at the selection, read once a frame through the desktop ribbon's own
    // three reads. Owned values, so the tiles can run commands while the frame is still up.
    let state = app.session.run("format.state", &json!({})).unwrap_or_default();
    let para = app.session.doc.para_at(&app.session.sel.focus).map(|p| app.session.doc.styles.resolve_para(&p.props));
    let align = para.as_ref().map(|r| r.align.visual(r.bidi)).unwrap_or_default();
    let lists = wordcraft_engine::cmd::para::list_buttons(&app.session.doc, para.as_ref().and_then(|r| r.numbering));
    Area::new(Id::new("wordcraft.mobile.command_sheet"))
        .order(Order::Foreground)
        .anchor(egui::Align2::RIGHT_BOTTOM, vec2(0.0, -(80.0 + NAV_INSET) + height * (1.0 - progress)))
        .show(&ctx, |ui| {
            ui.set_width(ui.ctx().content_rect().width());
            Frame::window(ui.style()).fill(t.ribbon).stroke(Stroke::new(1.0, t.border)).corner_radius(egui::CornerRadius::same(28)).inner_margin(egui::Margin::symmetric(EDGE as i8, 8)).show(ui, |ui| {
                ui.set_min_height(height);
                let (handle, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
                ui.painter().rect_filled(egui::Rect::from_center_size(handle.center(), vec2(32.0, 4.0)), 2.0, t.text_dim);
                // The sheets' title row is the dialogs': title at the left edge, a 48dp close at
                // the right, one BAR_H for both — the old 44×32 "×" was a third control language.
                if sheet_title(ui, tl!(tab), &t) {
                    set_sheet(ui, None);
                }
                ui.separator();
                // Three tiles across while each still has 88dp of its own (the brief's touch
                // floor), two below that. The tiles fill their column either way, so the sheet
                // never trails off into blank paper again.
                let width = ui.available_width();
                let cols = if (width - 2.0 * 8.0) / 3.0 >= 88.0 { 3 } else { 2 };
                let tile_w = ((width - 8.0 * (cols as f32 - 1.0)) / cols as f32).max(TOUCH);
                let label_sp = if cols >= 3 { 10.0 } else { 11.0 };
                let out = ScrollArea::vertical().max_height(height - 54.0).show(ui, |ui| {
                    for (row, (group, ids)) in groups.iter().enumerate() {
                        // One rhythm for the whole sheet: 8dp above a group, the caption, 8dp to
                        // its grid — nothing hangs a 4dp half-step off the P7 grid.
                        ui.add_space(if row == 0 { 4.0 } else { 8.0 });
                        ui.label(RichText::new(tl!(*group)).font(semibold(12.0)).color(t.text_dim));
                        ui.add_space(8.0);
                        egui::Grid::new(ui.id().with((tab, group))).num_columns(cols).spacing(vec2(8.0, 8.0)).show(ui, |ui| {
                            for (index, id) in ids.iter().enumerate() {
                                let close = command_button(ui, app, id, command_active(id, &state, align, lists), tile_w, label_sp, t);
                                if (index + 1) % cols == 0 {
                                    ui.end_row();
                                }
                                if close {
                                    set_sheet(ui, None);
                                }
                            }
                            if ids.len() % cols != 0 {
                                ui.end_row();
                            }
                        });
                    }
                });
                // More content than window: an 8dp scrim fading in over the bottom edge (one mesh,
                // vertex colours interpolated) says there is more below without a scrollbar eating
                // the width. In the sheet's own `ribbon` colour the wash would be invisible, so it
                // shades toward the text colour instead.
                let view = out.inner_rect;
                if out.content_size.y > view.height() + 1.0 {
                    let rect = egui::Rect::from_min_size(pos2(view.left(), view.bottom() - 8.0), vec2(view.width(), 8.0));
                    let mut fade = egui::Mesh::default();
                    for (x, y, color) in [
                        (rect.left(), rect.top(), t.text.gamma_multiply(0.0)),
                        (rect.right(), rect.top(), t.text.gamma_multiply(0.0)),
                        (rect.left(), rect.bottom(), t.text.gamma_multiply(0.08)),
                        (rect.right(), rect.bottom(), t.text.gamma_multiply(0.08)),
                    ] {
                        fade.colored_vertex(pos2(x, y), color);
                    }
                    fade.add_triangle(0, 1, 2);
                    fade.add_triangle(1, 3, 2);
                    ui.painter().add(fade);
                }
            });
        });
}

fn run(app: &mut WordApp, id: &str, params: serde_json::Value) {
    if let Err(error) = app.run(id, params) {
        app.status(error);
    }
}

/// A compact, middle-elided path that leaves the document name visible.
fn middle_ellipsis(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().take(512).collect();
    if chars.len() <= max_chars {
        return chars.into_iter().collect();
    }
    let kept = max_chars.saturating_sub(1);
    let head = kept / 2;
    let tail = kept.saturating_sub(head);
    chars[..head].iter().chain(std::iter::once(&'…')).chain(chars[chars.len().saturating_sub(tail)..].iter()).collect()
}

fn short_path(path: &str) -> String {
    let path: String = path.chars().take(512).collect();
    let parts: Vec<&str> = path.rsplit(['/', '\\']).filter(|part| !part.is_empty()).take(2).collect();
    let compact = match parts.as_slice() {
        [name, parent] => format!("…/{parent}/{name}"),
        [name] => (*name).to_owned(),
        [] => path,
        _ => path,
    };
    middle_ellipsis(&compact, 34)
}

fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days.saturating_add(719_468);
    let era = if z >= 0 { z } else { z.saturating_sub(146_096) } / 146_097;
    let doe = z.saturating_sub(era.saturating_mul(146_097));
    let yoe = (doe.saturating_sub(doe / 1_460).saturating_add(doe / 36_524).saturating_sub(doe / 146_096)) / 365;
    let year = yoe.saturating_add(era.saturating_mul(400));
    let doy = doe.saturating_sub(365_i64.saturating_mul(yoe)).saturating_sub(yoe / 4).saturating_add(yoe / 100);
    let month_part = (5_i64.saturating_mul(doy).saturating_add(2)) / 153;
    let day = doy.saturating_sub((153_i64.saturating_mul(month_part).saturating_add(2)) / 5).saturating_add(1) as u32;
    let month = (month_part + if month_part < 10 { 3 } else { -9 }) as u32;
    (year + if month <= 2 { 1 } else { 0 }, month, day)
}

/// A dependency-free relative stamp from the file's modification time.
fn recent_stamp(path: &str) -> String {
    let modified = std::fs::metadata(path).ok().and_then(|meta| meta.modified().ok());
    let age = modified.as_ref().and_then(|modified| modified.elapsed().ok());
    match age.map(|age| age.as_secs()) {
        Some(seconds) if seconds < 86_400 => tl!("Today").to_owned(),
        Some(seconds) if seconds < 172_800 => tl!("Yesterday").to_owned(),
        _ => modified
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|stamp| {
                let (_, month, day) = civil_date((stamp.as_secs() / 86_400).min(i64::MAX as u64) as i64);
                format!("{month}月{day}日")
            })
            .unwrap_or_else(|| "—".to_owned()),
    }
}

fn home_action(ui: &mut Ui, app: &mut WordApp, icon: &str, label: &str, filled: bool, command: &str, params: serde_json::Value, t: Tokens) {
    let response = ui.add_sized(
        vec2(ui.available_width().max(TOUCH), 56.0),
        Button::new("")
            .fill(if filled { t.accent } else { t.canvas })
            .stroke(Stroke::new(1.0, if filled { t.accent } else { t.border_strong }))
            .corner_radius(egui::CornerRadius::same(RADIUS as u8)),
    );
    // Accent is the primary action's alone; the outlined action beside it reads in `text`, so the
    // pair keeps the filled/outlined hierarchy instead of two accent blocks.
    let color = if filled { t.on_accent } else { t.text };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(response.rect.min.x + 28.0, response.rect.center().y), vec2(24.0, 24.0)),
        icon,
        color,
        color,
    );
    ui.painter().text(pos2(response.rect.min.x + 56.0, response.rect.center().y), Align2::LEFT_CENTER, label, semibold(16.0), color);
    if response.clicked() {
        run(app, command, params);
    }
}

fn recent_row(ui: &mut Ui, app: &mut WordApp, path: &str, t: Tokens) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width().max(TOUCH), 72.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, RADIUS, t.hover);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.min.x + 28.0, rect.center().y), vec2(20.0, 20.0)),
        "docOutline",
        t.text_dim,
        t.accent,
    );
    let name = std::path::Path::new(path).file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| path.to_owned());
    ui.painter().text(pos2(rect.min.x + 56.0, rect.min.y + 24.0), Align2::LEFT_CENTER, middle_ellipsis(&name, 30), regular(16.0), t.text);
    ui.painter().text(
        pos2(rect.min.x + 56.0, rect.min.y + 51.0),
        Align2::LEFT_CENTER,
        format!("{} · {}", recent_stamp(path), short_path(path)),
        regular(12.0),
        t.text_dim,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.max.x - 22.0, rect.center().y), vec2(12.0, 12.0)),
        "chevronRight",
        t.text_dim,
        t.accent,
    );
    if response.clicked() {
        run(app, "file.open", json!({"path": path}));
    }
}

/// Android's document-less start surface. The existing successful command hook persists recent
/// paths in `UiState`; this touch view deliberately presents only the newest eight.
pub fn show_home(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_home_top_bar")
        .exact_size(BAR_H + STATUS_INSET)
        .frame(Frame::NONE.fill(t.ribbon).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.add_space(STATUS_INSET);
            ui.horizontal(|ui| {
                ui.add_space(EDGE);
                ui.label(RichText::new(tl!("WordCraft")).font(semibold(20.0)).color(t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(EDGE);
                    top_button(ui, app, "settings", tl!("Options"), "ui.backstage", json!({"value": true, "page": "options", "pane": "general"}), true, &t);
                });
            });
        });
    egui::CentralPanel::default().frame(Frame::NONE.fill(t.canvas)).show(ui, |ui| {
        ui.add_space(EDGE);
        let recent: Vec<String> = app.ui.recent.iter().take(MOBILE_RECENT_LIMIT).cloned().collect();
        ui.horizontal(|ui| {
            ui.add_space(EDGE);
            ui.vertical(|ui| {
                home_action(ui, app, "plusFile", tl!("New Document"), true, "file.new", json!({}), t);
                ui.add_space(12.0);
                home_action(ui, app, "folderOutline", tl!("Open"), false, "ui.fileBrowser", json!({"purpose": "document"}), t);
                if recent.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(tl!("Documents you open will show up here.")).font(regular(12.0)).color(t.text_dim))
                    });
                    ui.add_space(24.0);
                    return;
                }
                ui.add_space(16.0);
                ui.label(RichText::new(tl!("Recent")).font(regular(12.0)).color(t.text_dim));
                ui.add_space(16.0);
                for (index, path) in recent.iter().enumerate() {
                    recent_row(ui, app, path, t);
                    if index + 1 < recent.len() {
                        let y = ui.cursor().top();
                        ui.painter().line_segment([pos2(ui.min_rect().min.x, y), pos2(ui.max_rect().max.x, y)], Stroke::new(1.0, t.border));
                    }
                }
                ui.add_space(24.0);
            });
            ui.add_space(EDGE);
        });
    });
}

/// Paint the Android-only ribbon replacement. Its state is egui-frame-local, while the selected
/// document tab remains `UiState::tab` through the normal `ui.tab` command.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("mobile_top_bar").exact_size(BAR_H + STATUS_INSET).frame(Frame::NONE.fill(t.ribbon).stroke(Stroke::new(1.0, t.border))).show(
        ui,
        |ui| {
            ui.add_space(STATUS_INSET);
            // One right-to-left stream, so buttons can never overlap: everything is laid out from
            // the right edge inward, and the title takes exactly the room that is left over once
            // the buttons and the back button's own [`EDGE`] margin are subtracted — the review's
            // clipped back button and lopsided title were both the old `- 32.0` guess.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(EDGE);
                top_button(ui, app, "more", tl!("Expand or collapse"), "ui.collapseRibbon", json!({}), true, &t);
                top_button(ui, app, "search", tl!("Search"), "ui.dialog", json!({"name": "find"}), true, &t);
                top_button(ui, app, "redo", tl!("Redo"), "edit.redo", json!({}), app.session.can_redo(), &t);
                top_button(ui, app, "undo", tl!("Undo"), "edit.undo", json!({}), app.session.can_undo(), &t);
                // What is left once the back button, its two spacings and the back button's own
                // [`EDGE`] margin are paid for — the exact remainder, so the stream ends on the
                // margin instead of eating into it.
                let spacing = ui.spacing().item_spacing.x;
                let title_width = (ui.available_width() - ICON_BTN - 2.0 * spacing - EDGE).max(48.0);
                let title = format!("{}{}", app.title_stem(), if app.session.dirty { " •" } else { "" });
                ui.add_sized(vec2(title_width, BAR_H), egui::Label::new(RichText::new(title).font(semibold(16.0))).truncate());
                top_button(ui, app, "arrowLeft", tl!("Close"), "file.close", json!({}), true, &t);
                // The stream's last reservation is the screen's left margin: the back button used
                // to start at x=0 and lose its left half to the bezel.
                ui.add_space(EDGE);
            });
        },
    );
    egui::Panel::bottom("mobile_tab_bar").exact_size(80.0 + NAV_INSET).frame(Frame::NONE.fill(t.tab_strip).stroke(Stroke::new(1.0, t.border))).show(
        ui,
        |ui| {
            // The gesture strip the bar reserves stays out of the drawing area: tabs, pills and
            // labels are measured against the height that is left, so the reserved strip is never
            // painted over, whatever the bar's exact height works out to.
            let height = (ui.available_height() - NAV_INSET).max(TOUCH);
            // The row insets by half an EDGE on both sides; the last tab used to kiss the screen's
            // right edge because the row ate the full width.
            ui.horizontal(|ui| {
                ui.add_space(EDGE / 2.0);
                let tab_width = ((ui.available_width() - EDGE / 2.0) / CORE_TABS.len() as f32).max(TOUCH);
                for tab in available_tabs(app) {
                    let open = sheet_tab(ui).as_deref() == Some(tab.as_str());
                    let (rect, response) = ui.allocate_exact_size(vec2(tab_width, height), Sense::click());
                    if open {
                        let pill = egui::Rect::from_center_size(rect.center_top() + vec2(0.0, 16.0), vec2(32.0, 32.0));
                        ui.painter().rect_filled(pill, 16.0, selection(&t));
                    }
                    icons::paint(
                        ui.painter(),
                        egui::Rect::from_center_size(rect.center_top() + vec2(0.0, 16.0), vec2(24.0, 24.0)),
                        tab_icon(&tab),
                        if open { t.accent } else { t.text },
                        t.accent,
                    );
                    // The label hangs above the strip it must clear, measured from the usable
                    // bottom, not from the bar's own edge.
                    ui.painter().text(
                        pos2(rect.center().x, rect.bottom() - 22.0),
                        egui::Align2::CENTER_CENTER,
                        tl!(&tab),
                        regular(12.0),
                        if open { t.accent } else { t.text_dim },
                    );
                    if response.clicked() {
                        if open {
                            set_sheet(ui, None);
                        } else {
                            set_sheet(ui, Some(&tab));
                        }
                        let _ = app.run("ui.tab", json!({"tab": tab}));
                    }
                }
                ui.add_space(EDGE / 2.0);
            });
        },
    );
    // The document itself. The desktop paints it through its own `CentralPanel` after the chrome;
    // the mobile bars above leave the rest of the frame to the same call, so a document created or
    // opened from the home screen shows up instead of leaving the surface at the clear colour.
    egui::CentralPanel::default().frame(Frame::NONE.fill(t.canvas)).show(ui, |ui| {
        crate::canvas::show(app, ui);
    });
    if let Some(tab) = sheet_tab(ui) {
        if available_tabs(app).iter().any(|available| available == &tab) {
            command_sheet(ui, app, &tab, t);
        } else {
            set_sheet(ui, None);
        }
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    //! Headless regression tests for the mobile shell. They run on the desktop through the
    //! `mobile` feature, which compiles this Android-only module there (`cargo test
    //! -p wordcraft-ui-egui --features mobile mobile::`), so a phone-layout break shows up in
    //! `cargo test` instead of on a device.

    // Clippy's test detection doesn't see through the `cfg(all(test, …))` gate above, so the
    // `clippy.toml` test exemptions don't apply on their own.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use egui::{Rect, epaint, pos2};
    use serde_json::json;

    use super::{CORE_TABS, NAV_INSET};
    use crate::{Services, WordApp};
    use wordcraft_engine::Session;

    /// Phone portrait, like the devices this port targets.
    const PHONE: egui::Vec2 = egui::Vec2::new(412.0, 892.0);

    /// TEMP(P9 diagnostic): dump painted texts and fills for the Home sheet.
    #[test]
    fn diag_sheet_layout_dump() {
        let mut app = phone_app();
        app.session.view.ruler = false;
        let mut h = egui_kittest::Harness::builder().with_size(PHONE).build_ui_state(
            |ui, app: &mut WordApp| {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("wordcraft.mobile.sheet"), "Home".to_owned()));
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        h.run_steps(2);
        h.state_mut().run("file.new", json!({})).expect("file.new runs");
        h.run_steps(6);
        for s in h.output().shapes.iter() {
            let r = s.shape.visual_bounding_rect();
            match &s.shape {
                epaint::Shape::Text(t) => println!("TEXT {:?} at {:?}", t.galley.text(), r),
                epaint::Shape::Rect(_) => println!("RECT {:?}", r),
                _ => {}
            }
        }
    }

    fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(PHONE).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        )
    }

    fn phone_app() -> WordApp {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        app.start_screen();
        // The Android shell launches this way (see apps/wordcraft-android); the tests exercise the
        // same configuration a phone runs.
        app.session.view.ruler = false;
        app.session.view.fit = "pageWidth".into();
        app
    }

    /// Every painted rectangle of the frame, in paint order.
    fn rects(h: &egui_kittest::Harness<'_, WordApp>) -> Vec<Rect> {
        h.output().shapes.iter().map(|s| s.shape.visual_bounding_rect()).collect()
    }

    /// A chrome bar: a frame-wide painted band about one bar tall. The harness wraps the app in an
    /// 8dp outer margin, so the bars are found in the output instead of assumed from the screen.
    fn bars(h: &egui_kittest::Harness<'_, WordApp>) -> Vec<Rect> {
        let wide = PHONE.x * 0.8;
        let mut bars: Vec<Rect> = rects(h).into_iter().filter(|r| r.width() >= wide && r.height() > 60.0 && r.height() < 140.0).collect();
        bars.sort_by(|a, b| a.top().total_cmp(&b.top()));
        bars
    }

    /// The document area: below the top bar, down to the tab bar when the edit shell shows one.
    fn canvas_strip(h: &egui_kittest::Harness<'_, WordApp>) -> Rect {
        let bars = bars(h);
        let top = bars.first().expect("the mobile top bar is painted").bottom();
        let bottom = bars.iter().map(|bar| bar.top()).filter(|t| *t > top).fold(PHONE.y, f32::min);
        Rect::from_min_max(pos2(0.0, top), pos2(PHONE.x, bottom))
    }

    /// Home renders, and New Document hands over to the edit shell with the document visible
    /// between the bars (the black screen after 新建文档: the shell drew its bars but nothing
    /// else, so the whole surface below the top bar stayed the renderer's clear colour).
    #[test]
    fn new_document_from_the_home_screen_paints_the_document() {
        let mut h = harness(phone_app());
        h.run_steps(6);
        assert!(!h.state().has_document(), "the home screen shows without a document");
        assert!(shapes_in(&h, canvas_strip(&h)).len() > 10, "the home surface paints nothing");

        h.state_mut().run("file.new", json!({})).expect("file.new runs");
        h.run_steps(6);
        assert!(h.state().has_document(), "file.new opens a document");

        let strip = canvas_strip(&h);
        let painted = shapes_in(&h, strip);
        assert!(painted.len() >= 8, "the edit shell paints almost nothing between the bars: {} shapes", painted.len());
        let page = painted.iter().any(|s| {
            let b = s.shape.visual_bounding_rect();
            b.width() > strip.width() * 0.4 && b.height() > 200.0
        });
        assert!(page, "no page-sized shape in the canvas strip; the document is not drawn");
    }

    /// The tab bar keeps its `NAV_INSET` gesture strip clear: nothing the bar draws may reach
    /// into the band the bar reserves above the screen edge.
    #[test]
    fn tab_bar_draws_clear_of_its_gesture_strip() {
        let mut h = harness(phone_app());
        h.run_steps(2);
        h.state_mut().run("file.new", json!({})).expect("file.new runs");
        h.run_steps(6);

        let bar = bars(&h).pop().expect("the tab bar is painted");
        let limit = bar.bottom() - NAV_INSET;
        // The bar's own frame legitimately fills the reserved strip; everything the bar draws
        // inside itself (tabs, pills, labels) must stay above it.
        let inside: Vec<Rect> = rects(&h).into_iter().filter(|r| bar.contains_rect(*r) && r.height() < bar.height() - NAV_INSET).collect();
        assert!(inside.len() > CORE_TABS.len(), "the tab bar paints nothing: {} shapes", inside.len());
        let over: Vec<Rect> = inside.iter().copied().filter(|r| r.bottom() > limit + 0.5).collect();
        assert!(over.is_empty(), "the tab bar paints {over:?} into its {NAV_INSET}dp gesture strip (must stay above {limit})");
        // The labels are there at all: five tab captions, one per core tab.
        let labels = inside.iter().filter(|r| r.height() < 30.0).count();
        assert!(labels >= CORE_TABS.len(), "the tab bar draws only {labels} label-sized shapes");
    }

    /// A tap on painted row `label`'s centre.
    fn tap(h: &mut egui_kittest::Harness<'_, WordApp>, at: egui::Pos2) {
        h.hover_at(at);
        h.drag_at(at);
        h.step();
        h.drop_at(at);
        h.step();
    }

    /// The painted rect of the first text shape containing `needle`.
    fn text_rect(h: &egui_kittest::Harness<'_, WordApp>, needle: &str) -> Option<Rect> {
        h.output().shapes.iter().find_map(|s| match &s.shape {
            epaint::Shape::Text(text) if text.galley.text().contains(needle) => Some(s.shape.visual_bounding_rect()),
            _ => None,
        })
    }

    /// The painted rect of the first text shape reading exactly `text` (a tab label next to a
    /// title that merely contains it, a button next to a field label).
    fn text_rect_exact(h: &egui_kittest::Harness<'_, WordApp>, text: &str) -> Option<Rect> {
        h.output().shapes.iter().find_map(|s| match &s.shape {
            epaint::Shape::Text(shape) if shape.galley.text() == text => Some(s.shape.visual_bounding_rect()),
            _ => None,
        })
    }

    /// The file browser opens as a full screen page with working rows, not the desktop's 620dp
    /// floating window: a tap enters a folder, a tap on a file fills the name field, and Cancel
    /// (the back arrow does the same) closes it without picking.
    #[test]
    fn the_file_browser_is_a_full_screen_page_with_working_rows() {
        let dir = std::env::temp_dir().join(format!("wordcraft-mobile-browser-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("stories")).unwrap();
        std::fs::write(dir.join("stories").join("inner.docx"), b"x").unwrap();
        std::fs::write(dir.join("novel.docx"), b"x").unwrap();

        let mut h = harness(phone_app());
        h.run_steps(2);
        h.state_mut().run("ui.fileBrowser", json!({"purpose": "document"})).expect("ui.fileBrowser runs");
        h.run_steps(2);
        // Same move the desktop location field makes, so the test does not depend on $HOME.
        h.state_mut().file_browser.open.as_mut().expect("the browser is open").navigate(dir.clone());
        h.run_steps(2);

        let bar = bars(&h).first().copied().expect("the browser bar is painted");
        assert!(bar.width() > PHONE.x * 0.9, "the browser bar is only {:?} wide, not a full screen page", bar);
        let covers = rects(&h).iter().any(|r| r.width() >= PHONE.x * 0.95 && r.height() >= PHONE.y * 0.95);
        assert!(covers, "the browser paints no full screen backdrop; it is a floating window, not a page");
        assert!(text_rect(&h, "novel.docx").is_some(), "the listing does not show the folder's files");

        let row = text_rect(&h, "stories").expect("the folder row is painted");
        assert!(row.top() < PHONE.y, "the rows are laid out off the screen at {:?}", row);
        tap(&mut h, row.center());
        h.run_steps(2);
        assert!(text_rect(&h, "inner.docx").is_some(), "tapping a folder did not open it");

        let row = text_rect(&h, "inner.docx").expect("the file row is painted");
        tap(&mut h, row.center());
        h.run_steps(2);
        assert_eq!(h.state().file_browser.open.as_ref().expect("still open").name, "inner.docx", "a tap puts the file in the name field");

        let cancel = text_rect(&h, "Cancel").expect("the Cancel button is painted");
        tap(&mut h, cancel.center());
        h.run_steps(2);
        assert!(h.state().file_browser.open.is_none(), "Cancel closed the browser without picking");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Shapes that reach into `area`: how much of the frame's drawing lands there.
    fn shapes_in<'a>(h: &'a egui_kittest::Harness<'a, WordApp>, area: Rect) -> Vec<&'a epaint::ClippedShape> {
        h.output().shapes.iter().filter(|s| s.shape.visual_bounding_rect().intersects(area)).collect()
    }

    /// The Find dialog fits the phone and keeps its buttons on one line: the sheet is narrow,
    /// and rows sized for the desktop's 620dp dialogs either overflow it or push their buttons
    /// off the sheet or under the tab bar's gesture strip. The Replace tab is the worst case
    /// (four buttons); 360dp is the narrow common phone width.
    #[test]
    fn the_find_dialog_fits_the_screen_and_keeps_its_buttons_on_one_line() {
        for width in [412.0, 360.0] {
            let size = egui::Vec2::new(width, PHONE.y);
            let app = phone_app();
            let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(
                |ui, app: &mut WordApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
            h.run_steps(2);
            h.state_mut().run("file.new", json!({})).expect("file.new runs");
            h.run_steps(2);
            // `edit.find` searches; the dialog is Advanced Find (the one Ctrl+H and Go To tab into).
            h.state_mut().run("edit.advancedFind", json!({})).expect("edit.advancedFind opens the Find dialog");
            h.run_steps(4);
            assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("find"), "the Find dialog is open");
            // To the Replace tab, where the button row is longest, and open Search Options.
            // Exact match: the window title "Find and Replace" contains the tab's word too.
            let replace = text_rect_exact(&h, "Replace").expect("the Replace tab is painted");
            tap(&mut h, replace.center());
            h.run_steps(2);
            let more = text_rect(&h, "Search Options").expect("Search Options is painted");
            tap(&mut h, more.center());
            h.run_steps(2);

            let mut buttons = Vec::new();
            for needle in ["Replace All", "Find Next", "Close"] {
                let r = text_rect(&h, needle).unwrap_or_else(|| panic!("the {needle} button is painted at {width}dp"));
                buttons.push((needle, r));
                assert!(r.left() >= 0.0 && r.right() <= width, "the {needle} button at {r:?} hangs off the {width}dp screen");
                assert!(r.bottom() < PHONE.y - NAV_INSET, "the {needle} button at {r:?} sits in the tab bar's gesture strip");
            }
            for (i, (a, ra)) in buttons.iter().enumerate() {
                for (b, rb) in buttons.iter().skip(i + 1) {
                    assert!(
                        (ra.center().y - rb.center().y).abs() < 4.0,
                        "at {width}dp the {a} and {b} buttons are not on one line: {ra:?} vs {rb:?}"
                    );
                }
            }
            // The field rows fit too: "Find what:" and its box are one row, not wrapped.
            let what = text_rect(&h, "Find what:").expect("the Find what label is painted");
            assert!(what.right() < width, "at {width}dp the Find what row overflows the screen at {what:?}");
        }
    }
}
