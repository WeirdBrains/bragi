//! The Theme screen (LF-33): pick a theme with a live preview of the whole window, light or dark
//! or the system's, and edit one: a colour per role with a hex box, the theme's palette, a
//! spectrum and lighter/darker steps, saved, duplicated, renamed, reset, imported and exported
//! in weirdnet-themes' TOML. Every action is a `bragi_core::Appearance` call, shared with the
//! terminal and phone apps; this only asks and shows.

use bragi_core::{Appearance, ModePref, Palette, Preference, ThemeRow};
use weirdnet_themes::{ThemeFile, Variant};

use super::*;

/// What the name/path box is asking for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    New,
    Duplicate,
    Rename,
    Import,
    Export,
}

impl Ask {
    fn label(self) -> &'static str {
        match self {
            Ask::New => "Name the new theme",
            Ask::Duplicate => "Name the copy",
            Ask::Rename => "New name",
            Ask::Import => "Path of a theme file to import",
            Ask::Export => "Path to write the theme to",
        }
    }
}

pub(super) struct Editing {
    pub name: String,
    pub file: ThemeFile,
    pub orig: ThemeFile,
    pub variant: Variant,
    pub role: usize,
}

#[derive(Default)]
pub(super) struct Themes {
    pub rows: Vec<ThemeRow>,
    pub sel: String,
    pub saved: Preference,
    pub mode: ModePref,
    pub edit: Option<Editing>,
    pub ask: Option<Ask>,
    pub msg: Option<String>,
    pub delete: Option<String>,
}

/// `#abc` / `#aabbcc` (any case, `#` optional) as `#rrggbb`.
pub(super) fn parse_hex(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches('#');
    let full: String = match s.len() {
        3 if s.chars().all(|c| c.is_ascii_hexdigit()) => s.chars().flat_map(|c| [c, c]).collect(),
        6 if s.chars().all(|c| c.is_ascii_hexdigit()) => s.to_string(),
        _ => return None,
    };
    Some(format!("#{}", full.to_ascii_lowercase()))
}

/// HSL (h in degrees, s and l in 0..1) as `#rrggbb`.
fn hsl(h: f32, s: f32, l: f32) -> String {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    weirdnet_themes::to_hex((q(r), q(g), q(b)))
}

impl BragiView {
    fn appearance(&self) -> Appearance {
        self.core.appearance()
    }

    /// Paints the window from `palette` (and the text boxes, which keep their own copy).
    pub(super) fn set_look(&mut self, palette: &Palette, cx: &mut Context<Self>) {
        let t = Theme::from_palette(palette);
        if t == self.theme {
            return;
        }
        self.theme = t;
        let mut boxes = vec![
            self.prompt.clone(),
            self.link_input.clone(),
            self.attach_input.clone(),
            self.confirm_input.clone(),
            self.models_query.clone(),
            self.theme_input.clone(),
            self.hex_input.clone(),
        ];
        boxes.extend([self.fields.name.clone(), self.fields.url.clone(), self.fields.key.clone(), self.fields.model.clone()]);
        for b in boxes {
            b.update(cx, |i, cx| i.set_theme(t, cx));
        }
        self.markdown.borrow_mut().clear();
        cx.notify();
    }

    /// The look of what the screen is showing: the pending edit, or the theme picked.
    fn show_look(&mut self, cx: &mut Context<Self>) {
        let a = self.appearance();
        let pal = match &self.themes.edit {
            Some(e) => Appearance::palette_of(&e.file, e.variant),
            None => match a.load(&self.themes.sel) {
                Ok(f) => Appearance::palette_of(&f, self.themes.mode.variant()),
                Err(_) => return,
            },
        };
        self.set_look(&pal, cx);
    }

    /// Puts back the look that is saved.
    fn restore_look(&mut self, cx: &mut Context<Self>) {
        let pal = self.appearance().palette();
        self.set_look(&pal, cx);
    }

    pub fn toggle_themes(&mut self, cx: &mut Context<Self>) {
        self.themes_open = !self.themes_open;
        self.settings_open = false;
        self.models_open = false;
        if self.themes_open {
            let a = self.appearance();
            self.themes.saved = a.preference();
            self.themes.mode = self.themes.saved.mode;
            self.themes.sel = self.themes.saved.theme.clone();
            self.themes.edit = None;
            self.themes.ask = None;
            self.themes.delete = None;
            self.themes.msg = None;
            self.reload_themes();
            self.show_look(cx);
        } else {
            self.themes.edit = None;
            self.restore_look(cx);
        }
        cx.notify();
    }

    fn reload_themes(&mut self) {
        self.themes.rows = self.appearance().rows().unwrap_or_default();
    }

    pub fn theme_pick(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.themes.edit.is_some() {
            return;
        }
        self.themes.sel = name.to_string();
        self.themes.msg = None;
        self.themes.delete = None;
        self.show_look(cx);
    }

    pub fn theme_mode(&mut self, mode: ModePref, cx: &mut Context<Self>) {
        self.themes.mode = mode;
        if let Some(e) = &mut self.themes.edit {
            if mode != ModePref::System {
                e.variant = mode.variant();
            }
            let role = e.role;
            self.select_role(role, cx);
        }
        self.show_look(cx);
    }

    /// Saves the picked theme and mode.
    pub fn theme_use(&mut self, cx: &mut Context<Self>) {
        let p = Preference { theme: self.themes.sel.clone(), mode: self.themes.mode };
        match self.appearance().set_preference(&p) {
            Ok(()) => {
                self.themes.msg = Some(format!("Using {} ({}).", self.themes.sel, p.mode.name()));
                self.themes.saved = p;
            }
            Err(e) => self.themes.msg = Some(e.message),
        }
        cx.notify();
    }

    // ---- the prompt (names and paths) ----

    pub fn theme_ask(&mut self, ask: Ask, cx: &mut Context<Self>) {
        let sel = self.themes.sel.clone();
        let text = match ask {
            Ask::New => weirdnet_themes::unique_name(&weirdnet_themes::UserDir::new(self.appearance().dir()), "my-theme"),
            Ask::Duplicate => weirdnet_themes::unique_name(&weirdnet_themes::UserDir::new(self.appearance().dir()), &format!("{sel}-copy")),
            Ask::Rename => sel.clone(),
            Ask::Import => String::new(),
            Ask::Export => format!("{sel}.toml"),
        };
        if ask == Ask::Rename && self.themes.rows.iter().any(|r| r.name == sel && r.builtin) {
            self.themes.msg = Some("A built-in can't be renamed; duplicate it instead.".into());
            return cx.notify();
        }
        self.themes.ask = Some(ask);
        self.themes.msg = None;
        self.theme_input.update(cx, |i, cx| i.set_text(&text, cx));
        cx.notify();
    }

    pub fn theme_ask_cancel(&mut self, cx: &mut Context<Self>) {
        self.themes.ask = None;
        cx.notify();
    }

    pub fn theme_ask_submit(&mut self, cx: &mut Context<Self>) {
        let Some(ask) = self.themes.ask else { return };
        let text = self.theme_input.read(cx).text().trim().to_string();
        if text.is_empty() {
            self.themes.msg = Some("Type something first.".into());
            return cx.notify();
        }
        let a = self.appearance();
        let sel = self.themes.sel.clone();
        let done = match ask {
            Ask::New | Ask::Duplicate => a.duplicate(&sel, Some(&text)).map(|n| {
                self.themes.msg = Some(format!("Made {n}."));
                self.themes.sel = n.clone();
                Some(n)
            }),
            Ask::Rename => a.rename(&sel, &text).map(|()| {
                self.themes.msg = Some(format!("Renamed to {text}."));
                self.themes.sel = text.clone();
                self.themes.saved = a.preference();
                None
            }),
            Ask::Import => a.import_file(std::path::Path::new(&text), None).map(|n| {
                self.themes.msg = Some(format!("Imported {n}."));
                self.themes.sel = n;
                None
            }),
            Ask::Export => a.export_file(&sel, std::path::Path::new(&text)).map(|()| {
                self.themes.msg = Some(format!("Wrote {text}."));
                None
            }),
        };
        match done {
            Ok(edit) => {
                self.themes.ask = None;
                self.reload_themes();
                match (ask, edit) {
                    (Ask::New, Some(n)) => self.theme_edit_named(&n, cx),
                    _ => self.show_look(cx),
                }
            }
            Err(e) => self.themes.msg = Some(e.message),
        }
        cx.notify();
    }

    pub fn theme_delete(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        let name = self.themes.sel.clone();
        if !confirmed {
            let r = self.themes.rows.iter().find(|r| r.name == name);
            if r.is_some_and(|r| r.builtin && !r.user_file) {
                self.themes.msg = Some("A built-in can't be deleted.".into());
            } else {
                self.themes.delete = Some(name);
            }
            return cx.notify();
        }
        self.themes.delete = None;
        match self.appearance().delete(&name) {
            Ok(()) => {
                self.themes.msg = Some(format!("Deleted {name}."));
                self.themes.saved = self.appearance().preference();
                self.reload_themes();
                if !self.themes.rows.iter().any(|r| r.name == name) {
                    self.themes.sel = self.themes.saved.theme.clone();
                }
                self.show_look(cx);
            }
            Err(e) => self.themes.msg = Some(e.message),
        }
        cx.notify();
    }

    pub fn theme_reset(&mut self, cx: &mut Context<Self>) {
        let name = self.themes.sel.clone();
        let edited = self.themes.rows.iter().any(|r| r.name == name && r.builtin && r.user_file);
        if !edited {
            self.themes.msg = Some("Only an edited built-in can be reset.".into());
            return cx.notify();
        }
        // Dropping the edits file puts the shipped theme back.
        match self.appearance().delete(&name) {
            Ok(()) => self.themes.msg = Some(format!("{name} is as it ships.")),
            Err(e) => self.themes.msg = Some(e.message),
        }
        self.reload_themes();
        self.show_look(cx);
    }

    // ---- the editor ----

    pub fn theme_edit(&mut self, cx: &mut Context<Self>) {
        let sel = self.themes.sel.clone();
        self.theme_edit_named(&sel, cx);
    }

    fn theme_edit_named(&mut self, name: &str, cx: &mut Context<Self>) {
        let a = self.appearance();
        let target = match a.editable(name) {
            Ok(t) => t,
            Err(e) => {
                self.themes.msg = Some(e.message);
                return cx.notify();
            }
        };
        let Ok(file) = a.load(&target) else { return };
        if target != name {
            self.themes.msg = Some(format!("{name} ships with Bragi, so you're editing a copy: {target}."));
            self.reload_themes();
            self.themes.sel = target.clone();
        }
        let variant = self.themes.mode.variant();
        self.themes.edit = Some(Editing { name: target, orig: file.clone(), file, variant, role: 0 });
        self.select_role(0, cx);
        self.show_look(cx);
    }

    fn role_hex(&self, i: usize) -> String {
        let Some(e) = &self.themes.edit else { return String::new() };
        let role = bragi_core::appearance::roles()[i];
        Appearance::palette_of(&e.file, e.variant).get(role).unwrap_or("#000000").to_string()
    }

    pub fn select_role(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(e) = &mut self.themes.edit {
            e.role = i;
        }
        let hex = self.role_hex(i);
        self.hex_input.update(cx, |t, cx| t.set_text(&hex, cx));
        cx.notify();
    }

    fn put_role(&mut self, hex: &str, cx: &mut Context<Self>) {
        if let Some(e) = &mut self.themes.edit {
            let role = bragi_core::appearance::roles()[e.role];
            let _ = e.file.variant_mut(e.variant).set_role(role, hex);
        }
        self.show_look(cx);
    }

    /// The hex box was typed in: a valid colour is live.
    pub fn hex_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.hex_input.read(cx).text().to_string();
        if let Some(h) = parse_hex(&text) {
            self.put_role(&h, cx);
        }
        cx.notify();
    }

    /// A swatch, a spectrum chip or a step was clicked.
    pub fn role_to(&mut self, hex: &str, cx: &mut Context<Self>) {
        self.hex_input.update(cx, |t, cx| t.set_text(hex, cx));
        self.put_role(hex, cx);
    }

    pub fn role_step(&mut self, toward_white: bool, cx: &mut Context<Self>) {
        let Some(e) = &self.themes.edit else { return };
        let cur = weirdnet_themes::parse_hex(&self.role_hex(e.role)).unwrap_or((128, 128, 128));
        let to = if toward_white { (255, 255, 255) } else { (0, 0, 0) };
        let h = weirdnet_themes::to_hex(weirdnet_themes::mix(cur, to, 0.1));
        self.role_to(&h, cx);
    }

    pub fn role_undo(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &mut self.themes.edit else { return };
        let role = bragi_core::appearance::roles()[e.role];
        match e.orig.variant(e.variant).roles.get(role).cloned() {
            Some(v) => {
                e.file.variant_mut(e.variant).roles.insert(role.into(), v);
            }
            None => {
                let _ = e.file.variant_mut(e.variant).clear_role(role);
            }
        }
        let i = e.role;
        self.select_role(i, cx);
        self.show_look(cx);
    }

    pub fn role_clear(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &mut self.themes.edit else { return };
        let role = bragi_core::appearance::roles()[e.role];
        if e.file.variant_mut(e.variant).clear_role(role).is_err() {
            self.themes.msg = Some("Only an optional role (panels, code, diffs, links, hints) can be cleared.".into());
            return cx.notify();
        }
        let i = e.role;
        self.select_role(i, cx);
        self.show_look(cx);
    }

    pub fn edit_variant(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &mut self.themes.edit else { return };
        e.variant = e.variant.toggle();
        let i = e.role;
        self.select_role(i, cx);
        self.show_look(cx);
    }

    pub fn edit_dirty(&self) -> bool {
        self.themes.edit.as_ref().is_some_and(|e| e.file != e.orig)
    }

    pub fn edit_save(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &self.themes.edit else { return };
        let a = self.appearance();
        let name = e.name.clone();
        for v in [Variant::Dark, Variant::Light] {
            let (now, was) = (e.file.variant(v), e.orig.variant(v));
            let mut edits = Vec::new();
            for r in bragi_core::appearance::roles() {
                match (now.roles.get(r), was.roles.get(r)) {
                    (x, y) if x == y => {}
                    (Some(val), _) => edits.push((r.to_string(), val.clone())),
                    (None, Some(_)) => edits.push((r.to_string(), String::new())),
                    _ => {}
                }
            }
            if !edits.is_empty()
                && let Err(err) = a.apply(&name, v, &edits)
            {
                self.themes.msg = Some(err.message);
                return cx.notify();
            }
        }
        if let Ok(f) = a.load(&name)
            && let Some(e) = &mut self.themes.edit
        {
            let warn = weirdnet_themes::contrast_warnings(f.variant(e.variant));
            e.orig = f.clone();
            e.file = f;
            self.themes.msg = Some(match warn.first() {
                Some(w) => format!("Saved {name}. Check: {w}"),
                None => format!("Saved {name}."),
            });
        }
        self.reload_themes();
        cx.notify();
    }

    /// Back to the list, dropping what wasn't saved.
    pub fn edit_close(&mut self, cx: &mut Context<Self>) {
        self.themes.edit = None;
        self.show_look(cx);
        cx.notify();
    }

    // ---- drawing ----

    fn note(&self, text: impl Into<SharedString>, color: u32) -> gpui::Div {
        div().text_size(px(12.)).text_color(rgb(color)).child(text.into())
    }

    fn chip(&self, id: impl Into<gpui::ElementId>, hex: &str, size: f32) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        let c = weirdnet_themes::parse_hex(hex).map(|(r, g, b)| u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)).unwrap_or(0);
        div()
            .id(id)
            .w(px(size))
            .h(px(size))
            .rounded(px(4.))
            .bg(rgb(c))
            .border_1()
            .border_color(rgb(t.border))
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
    }

    pub(super) fn themes_screen(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let st = &self.themes;
        let a = self.appearance();
        let editing = st.edit.is_some();

        // The list.
        let mut list = div().flex().flex_col().gap(px(2.));
        for r in &st.rows {
            let on = r.name == st.sel;
            let saved = r.name == st.saved.theme;
            let pal = a.palette_for(&r.name, st.mode.variant()).ok();
            let mut dots = div().flex().flex_row().gap(px(3.));
            if let Some(p) = &pal {
                for role in ["bg", "accent", "fg", "up"] {
                    dots = dots.child(div().w(px(10.)).h(px(10.)).rounded(px(5.)).bg(rgb(p.rgb(role))).border_1().border_color(rgb(t.border)));
                }
            }
            let tag = match (r.builtin, r.user_file, &r.error) {
                (_, _, Some(_)) => " · broken",
                (true, true, _) => " · edited",
                (true, false, _) => "",
                _ => " · yours",
            };
            let name = r.name.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("theme-{}", r.name)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .when(on, |d| d.bg(rgb(t.selection)))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.85))
                    .on_click(cx.listener(move |v, _, _, cx| v.theme_pick(&name, cx)))
                    .child(div().w(px(10.)).text_size(px(11.)).text_color(rgb(t.success)).child(if saved { "●" } else { "" }))
                    .child(div().flex_1().text_size(px(13.)).child(format!("{}{tag}", r.display_name)))
                    .child(dots),
            );
        }

        let mode_btn = |id: &'static str, label: &'static str, m: ModePref| {
            self.button(id, label, st.mode == m).h(px(28.)).px(px(10.)).on_click(cx.listener(move |v, _, _, cx| v.theme_mode(m, cx)))
        };
        let modes = div()
            .flex()
            .flex_row()
            .gap(px(6.))
            .child(mode_btn("mode-dark", "Dark", ModePref::Dark))
            .child(mode_btn("mode-light", "Light", ModePref::Light))
            .child(mode_btn("mode-system", "System", ModePref::System));

        let act = |id: &'static str, label: &'static str, primary: bool| self.button(id, label, primary).h(px(28.)).px(px(10.));
        let actions = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(6.))
            .child(act("theme-use", "Use", true).on_click(cx.listener(|v, _, _, cx| v.theme_use(cx))))
            .child(act("theme-edit", "Edit colours", false).on_click(cx.listener(|v, _, _, cx| v.theme_edit(cx))))
            .child(act("theme-new", "New", false).on_click(cx.listener(|v, _, _, cx| v.theme_ask(Ask::New, cx))))
            .child(act("theme-dup", "Duplicate", false).on_click(cx.listener(|v, _, _, cx| v.theme_ask(Ask::Duplicate, cx))))
            .child(act("theme-rename", "Rename", false).on_click(cx.listener(|v, _, _, cx| v.theme_ask(Ask::Rename, cx))))
            .child(act("theme-reset", "Reset", false).on_click(cx.listener(|v, _, _, cx| v.theme_reset(cx))))
            .child(act("theme-delete", "Delete", false).text_color(rgb(t.error)).on_click(cx.listener(|v, _, _, cx| v.theme_delete(false, cx))))
            .child(act("theme-import", "Import…", false).on_click(cx.listener(|v, _, _, cx| v.theme_ask(Ask::Import, cx))))
            .child(act("theme-export", "Export…", false).on_click(cx.listener(|v, _, _, cx| v.theme_ask(Ask::Export, cx))));

        let left = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .flex_none()
            .w(px(340.))
            .child(self.note("THEMES", t.text_secondary))
            .child(list)
            .child(self.note("MODE", t.text_secondary))
            .child(modes)
            .child(actions);

        let right = if editing { self.theme_editor(cx).into_any_element() } else { self.theme_preview(cx).into_any_element() };

        let mut screen = div()
            .id("themes")
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .flex_1()
            .overflow_y_scroll()
            .child(div().text_size(px(18.)).child("Theme"))
            .child(div().flex().flex_row().gap(px(24.)).items_start().child(left).child(right));
        if let Some(ask) = st.ask {
            screen = screen.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(t.surface))
                    .child(div().text_size(px(13.)).child(ask.label()))
                    .child(
                        div()
                            .flex_1()
                            .px(px(8.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .bg(rgb(t.background))
                            .border_1()
                            .border_color(rgb(t.border))
                            .child(self.theme_input.clone()),
                    )
                    .child(self.button("ask-ok", "OK", true).on_click(cx.listener(|v, _, _, cx| v.theme_ask_submit(cx))))
                    .child(self.button("ask-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.theme_ask_cancel(cx)))),
            );
        }
        if let Some(name) = &st.delete {
            screen = screen.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(t.surface))
                    .child(div().flex_1().text_size(px(13.)).child(format!("Delete {name}? Its file goes from this device.")))
                    .child(self.button("delete-yes", "Delete", true).bg(rgb(t.error)).on_click(cx.listener(|v, _, _, cx| v.theme_delete(true, cx))))
                    .child(self.button("delete-no", "Keep", false).on_click(cx.listener(|v, _, _, cx| {
                        v.themes.delete = None;
                        cx.notify();
                    }))),
            );
        }
        if let Some(m) = &st.msg {
            screen = screen.child(self.note(m.clone(), t.text_secondary));
        }
        screen.child(self.button("themes-done", "Done", false).w(px(80.)).on_click(cx.listener(|v, _, _, cx| v.toggle_themes(cx))))
    }

    /// A sample of the window in the colours being previewed (the window already is).
    fn theme_preview(&self, _cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .flex_1()
            .p(px(14.))
            .rounded(px(10.))
            .bg(rgb(t.surface))
            .border_1()
            .border_color(rgb(t.border))
            .child(self.note("PREVIEW", t.text_secondary))
            .child(div().self_end().px(px(12.)).py(px(8.)).rounded(px(10.)).bg(rgb(t.selection)).text_size(px(13.)).child("How do I undo a rebase?"))
            .child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(10.))
                    .bg(rgb(t.surface_variant))
                    .text_size(px(13.))
                    .child("Use git reflog to find where the branch was, then reset to it."),
            )
            .child(
                div()
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .bg(rgb(t.background))
                    .border_1()
                    .border_color(rgb(t.border))
                    .font_family(self.mono.clone())
                    .text_size(px(12.))
                    .child("$ git reflog\n$ git reset --hard HEAD@{2}"),
            )
            .child(self.note("secondary text · a hint", t.text_secondary))
            .child(div().text_size(px(12.)).text_color(rgb(t.link)).child("a link"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(14.))
                    .text_size(px(12.))
                    .child(div().text_color(rgb(t.success)).child("✓ done"))
                    .child(div().text_color(rgb(t.warning)).child("! needs a look"))
                    .child(div().text_color(rgb(t.error)).child("✗ error")),
            )
            .child(
                div().flex().flex_row().gap(px(8.)).child(self.button("pv-primary", "Primary", true)).child(self.button("pv-plain", "Plain", false)),
            )
    }

    fn theme_editor(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let e = self.themes.edit.as_ref().expect("editing");
        let pal = Appearance::palette_of(&e.file, e.variant);
        let roles = bragi_core::appearance::roles();
        let role = roles[e.role];
        let mut rows = div().flex().flex_col().gap(px(1.)).w(px(250.));
        for (i, r) in roles.iter().enumerate() {
            let hex = pal.get(r).unwrap_or("#000000").to_string();
            let auto = !e.file.variant(e.variant).roles.contains_key(*r);
            let changed = e.file.variant(e.variant).roles.get(*r) != e.orig.variant(e.variant).roles.get(*r);
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("role-{r}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(5.))
                    .when(i == e.role, |d| d.bg(rgb(t.selection)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |v, _, _, cx| v.select_role(i, cx)))
                    .child(div().w(px(16.)).h(px(16.)).rounded(px(3.)).border_1().border_color(rgb(t.border)).bg(rgb(pal.rgb(r))))
                    .child(div().flex_1().text_size(px(12.)).child(format!("{r}{}", if changed { " •" } else { "" })))
                    .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(if auto { format!("{hex} auto") } else { hex })),
            );
        }

        // The picker for the role under the cursor.
        let mut palette = div().flex().flex_row().flex_wrap().gap(px(4.));
        for (k, (name, hex)) in e.file.variant(e.variant).swatches().into_iter().enumerate() {
            let h = hex.clone();
            palette = palette.child(
                self.chip(SharedString::from(format!("sw-{k}-{name}")), &hex, 20.).on_click(cx.listener(move |v, _, _, cx| v.role_to(&h, cx))),
            );
        }
        let mut spectrum = div().flex().flex_row().flex_wrap().gap(px(3.));
        for (row, l) in [0.35f32, 0.55, 0.75].into_iter().enumerate() {
            for k in 0..12 {
                let hex = hsl(k as f32 * 30.0, 0.7, l);
                let h = hex.clone();
                spectrum = spectrum.child(
                    self.chip(SharedString::from(format!("hue-{row}-{k}")), &hex, 18.).on_click(cx.listener(move |v, _, _, cx| v.role_to(&h, cx))),
                );
            }
        }
        let typed = self.hex_input.read(cx).text().to_string();
        let bad = parse_hex(&typed).is_none();
        let step = |id: &'static str, label: &'static str| self.button(id, label, false).h(px(26.)).px(px(8.));
        let panel = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .flex_1()
            .p(px(12.))
            .rounded(px(10.))
            .bg(rgb(t.surface))
            .border_1()
            .border_color(rgb(t.border))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(14.)).child(format!("{} · {}", e.name, e.variant.name())))
                    .child(div().flex_1())
                    .child(
                        step("edit-variant", if e.variant == Variant::Dark { "Edit light" } else { "Edit dark" })
                            .on_click(cx.listener(|v, _, _, cx| v.edit_variant(cx))),
                    ),
            )
            .child(self.note(format!("{role} — {}", bragi_core::appearance::role_label(role)), t.text_secondary))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(div().w(px(28.)).h(px(28.)).rounded(px(5.)).border_1().border_color(rgb(t.border)).bg(rgb(pal.rgb(role))))
                    .child(
                        div()
                            .w(px(120.))
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .bg(rgb(t.background))
                            .border_1()
                            .border_color(rgb(if bad { t.error } else { t.border }))
                            .child(self.hex_input.clone()),
                    )
                    .child(step("step-lighter", "Lighter").on_click(cx.listener(|v, _, _, cx| v.role_step(true, cx))))
                    .child(step("step-darker", "Darker").on_click(cx.listener(|v, _, _, cx| v.role_step(false, cx)))),
            )
            .child(self.note("PALETTE", t.text_secondary))
            .child(palette)
            .child(self.note("SPECTRUM", t.text_secondary))
            .child(spectrum)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .child(step("role-undo", "Undo role").on_click(cx.listener(|v, _, _, cx| v.role_undo(cx))))
                    .child(step("role-clear", "Back to auto").on_click(cx.listener(|v, _, _, cx| v.role_clear(cx)))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .child(
                        self.button("edit-save", if self.edit_dirty() { "Save" } else { "Saved" }, self.edit_dirty())
                            .on_click(cx.listener(|v, _, _, cx| v.edit_save(cx))),
                    )
                    .child(
                        self.button("edit-close", if self.edit_dirty() { "Discard" } else { "Close" }, false)
                            .on_click(cx.listener(|v, _, _, cx| v.edit_close(cx))),
                    ),
            );
        div().flex().flex_row().gap(px(16.)).flex_1().items_start().child(rows).child(panel)
    }
}
