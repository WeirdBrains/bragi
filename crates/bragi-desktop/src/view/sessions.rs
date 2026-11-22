//! The Sessions section of the sidebar and the Endpoints panel. Sessions sit in nested folders:
//! click a session to open it, click a folder to fold it, drag a session or folder onto a folder
//! or the SESSIONS header to move it (or pick it and press Move, then click where it goes). The
//! name box above the tree creates and renames. Every change is one `Core` call; the tree is
//! drawn from `AppState.sessions` in the order the core gives.

use bragi_core::{DEFAULT_SESSION, EndpointHealth, EndpointKind, Rebind, RouteKind, Row, Target};
use gpui::{Context, IntoElement, Render, SharedString, Window, div, prelude::*, px, rgb};

use super::BragiView;
use crate::theme::Theme;

/// What the name box above the tree does on Enter.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum NameEdit {
    NewSession { folder: Option<String> },
    NewFolder { parent: Option<String> },
    RenameSession(String),
    RenameFolder(String),
}

/// A row being dragged.
#[derive(Clone)]
pub(super) struct Dragged {
    row: Row,
    name: String,
    theme: Theme,
}

impl Render for Dragged {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .px(px(10.))
            .py(px(5.))
            .rounded(px(6.))
            .bg(rgb(t.surface_variant))
            .border_1()
            .border_color(rgb(t.accent))
            .text_size(px(13.))
            .text_color(rgb(t.text_primary))
            .child(self.name.clone())
    }
}

impl BragiView {
    fn try_core(&mut self, r: bragi_core::Result<()>) -> bool {
        match r {
            Ok(()) => {
                self.notice = None;
                true
            }
            Err(e) => {
                self.notice = Some(e.message);
                false
            }
        }
    }

    // ---- session and folder actions (buttons, drops and the scenario come here) -------------

    pub fn begin_edit(&mut self, edit: NameEdit, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let initial = match &edit {
            NameEdit::RenameSession(id) => self.state.sessions.get(id).map(|s| s.name.clone()).unwrap_or_default(),
            NameEdit::RenameFolder(id) => self.state.sessions.folder(id).map(|f| f.name.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        let hint = match &edit {
            NameEdit::NewSession { .. } => "New session name…",
            NameEdit::NewFolder { .. } => "New folder name…",
            _ => "New name…",
        };
        if matches!(edit, NameEdit::NewSession { .. }) {
            self.new_target = None;
        }
        self.name_input.update(cx, |i, cx| {
            i.configure(hint, false, cx);
            i.set_text(&initial, cx);
        });
        if let Some(w) = window {
            w.focus(&gpui::Focusable::focus_handle(self.name_input.read(cx), cx), cx);
        }
        self.editing = Some(edit);
        cx.notify();
    }

    pub fn set_name(&mut self, text: &str, cx: &mut Context<Self>) {
        self.name_input.update(cx, |i, cx| i.set_text(text, cx));
    }

    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    /// Enter in the name box: create or rename.
    pub fn commit_name(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.editing.clone() else { return };
        let name = self.name_input.read(cx).text().trim().to_string();
        let r = match edit {
            NameEdit::NewSession { folder } => {
                let target = self.new_target.clone().map_or(Target::Weirdnet, |id| Target::Endpoint { id });
                self.core.create_session(&name, target, None, folder).map(|s| Some(s.id))
            }
            NameEdit::NewFolder { parent } => self.core.create_folder(&name, parent).map(|f| {
                self.picked = Some(Row::Folder(f.id));
                None
            }),
            NameEdit::RenameSession(id) => self.core.rename_session(&id, &name).map(|_| None),
            NameEdit::RenameFolder(id) => self.core.rename_folder(&id, &name).map(|_| None),
        };
        match r {
            Ok(opened) => {
                self.editing = None;
                self.notice = None;
                if let Some(id) = opened {
                    self.switch_session(&id, cx);
                }
            }
            Err(e) => self.notice = Some(e.message),
        }
        cx.notify();
    }

    /// Opens a session: its own history, its own model, its own target.
    pub fn switch_session(&mut self, id: &str, cx: &mut Context<Self>) {
        let r = self.core.switch_session(id);
        if self.try_core(r) {
            self.state = self.core.state();
            self.picked = Some(Row::Session(id.to_string()));
            self.current = None;
            self.model = None;
            self.reload();
        }
        cx.notify();
    }

    /// A click on a folder: folds or unfolds it, or, while moving, moves there.
    pub fn click_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.moving.is_some() {
            self.move_into(Some(id.to_string()), cx);
            return;
        }
        let collapsed = self.state.sessions.folder(id).is_some_and(|f| f.collapsed);
        let r = self.core.set_folder_collapsed(id, !collapsed);
        self.try_core(r);
        self.picked = Some(Row::Folder(id.to_string()));
        cx.notify();
    }

    /// Move mode: the picked row moves to the next folder (or SESSIONS, the top level) clicked.
    pub fn start_move(&mut self, cx: &mut Context<Self>) {
        self.moving = self.picked.clone();
        cx.notify();
    }

    pub fn move_into(&mut self, folder: Option<String>, cx: &mut Context<Self>) {
        if let Some(row) = self.moving.take() {
            self.move_row(row, folder, cx);
        }
        cx.notify();
    }

    pub fn move_row(&mut self, row: Row, folder: Option<String>, cx: &mut Context<Self>) {
        let r = match &row {
            Row::Session(id) => self.core.move_session(id, folder),
            Row::Folder(id) if folder.as_deref() == Some(id.as_str()) => Ok(()),
            Row::Folder(id) => self.core.move_folder(id, folder),
        };
        self.try_core(r);
        cx.notify();
    }

    /// Delete on the picked row: a session goes at once (its conversations move to the Weirdnet
    /// session), a folder asks first.
    pub fn delete_picked(&mut self, cx: &mut Context<Self>) {
        match self.picked.clone() {
            Some(Row::Session(id)) => {
                let r = self.core.delete_session(&id);
                if self.try_core(r) {
                    self.picked = None;
                    self.current = None;
                    self.model = None;
                    self.reload();
                }
            }
            Some(Row::Folder(id)) => self.confirm_folder = Some(id),
            None => {}
        }
        cx.notify();
    }

    /// The answer to "Delete this folder?": its sessions and folders move up a level.
    pub fn confirm_delete_folder(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(id) = self.confirm_folder.take()
            && yes
        {
            let r = self.core.delete_folder(&id);
            if self.try_core(r) {
                self.picked = None;
            }
        }
        cx.notify();
    }

    /// The next target in turn (Weirdnet, then each endpoint): for the session being created,
    /// or for the picked session.
    pub fn cycle_target(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<Option<String>> = std::iter::once(None).chain(self.state.endpoints.iter().map(|e| Some(e.id.clone()))).collect();
        let next = |cur: Option<&str>| {
            let i = ids.iter().position(|x| x.as_deref() == cur).unwrap_or(0);
            ids[(i + 1) % ids.len()].clone()
        };
        if matches!(self.editing, Some(NameEdit::NewSession { .. })) {
            self.new_target = next(self.new_target.as_deref());
        } else if let Some(Row::Session(id)) = &self.picked
            && let Some(s) = self.state.sessions.get(id)
        {
            let cur = match &s.target {
                Target::Weirdnet => None,
                Target::Endpoint { id } => Some(id.as_str()),
            };
            let target = next(cur).map_or(Target::Weirdnet, |id| Target::Endpoint { id });
            let r = self.core.set_session_target(&s.id.clone(), target);
            if self.try_core(r) && s.id == self.state.sessions.active {
                self.model = None;
            }
        }
        cx.notify();
    }

    // ---- endpoints ---------------------------------------------------------------------------

    pub fn toggle_endpoints(&mut self, cx: &mut Context<Self>) {
        self.endpoints_open = !self.endpoints_open;
        cx.notify();
    }

    /// Checks the typed endpoint against its /v1/models and adds it. The token box is cleared
    /// either way; the token lives only in the core's private file.
    pub fn add_endpoint(&mut self, cx: &mut Context<Self>) {
        if self.ep_busy {
            return;
        }
        let name = self.ep_name.read(cx).text().to_string();
        let url = self.ep_url.read(cx).text().to_string();
        let token = self.ep_token.read(cx).text().to_string();
        self.ep_token.update(cx, |i, cx| i.set_text("", cx));
        self.ep_busy = true;
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.add_endpoint(&name, &url, &token).await });
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                v.ep_busy = false;
                match r {
                    Ok(Ok(_)) => {
                        v.notice = None;
                        v.ep_name.update(cx, |i, cx| i.set_text("", cx));
                        v.ep_url.update(cx, |i, cx| i.set_text("", cx));
                    }
                    Ok(Err(e)) => v.notice = Some(e.message),
                    Err(e) => v.notice = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the form on an endpoint to edit it: its name and URL filled in, the token box empty
    /// (the saved token is never read back; typing one replaces it).
    pub fn begin_edit_endpoint(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(e) = self.state.endpoints.iter().find(|e| e.id == id).cloned() else { return };
        self.ep_name.update(cx, |i, cx| i.set_text(&e.name, cx));
        self.ep_url.update(cx, |i, cx| i.set_text(&e.url, cx));
        self.ep_token.update(cx, |i, cx| {
            i.configure("New token (empty keeps the saved one)", true, cx);
            i.set_text("", cx)
        });
        self.ep_model = e.model.clone();
        self.ep_fallback = e.fallback;
        self.ep_tested = None;
        self.ep_editing = Some(e.id);
        cx.notify();
    }

    pub fn cancel_edit_endpoint(&mut self, cx: &mut Context<Self>) {
        self.ep_editing = None;
        self.ep_tested = None;
        for i in [&self.ep_name, &self.ep_url, &self.ep_token] {
            i.update(cx, |i, cx| i.set_text("", cx));
        }
        self.ep_token.update(cx, |i, cx| i.configure("Bearer token", true, cx));
        cx.notify();
    }

    /// The next default model in turn for the endpoint being edited (`None`: its first).
    pub fn cycle_endpoint_model(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.ep_editing.as_ref().and_then(|id| self.state.endpoints.iter().find(|e| &e.id == id)) else { return };
        let ids: Vec<Option<String>> = std::iter::once(None).chain(e.models.iter().map(|m| Some(m.id.clone()))).collect();
        let i = ids.iter().position(|x| *x == self.ep_model).unwrap_or(0);
        self.ep_model = ids[(i + 1) % ids.len()].clone();
        cx.notify();
    }

    pub fn toggle_endpoint_fallback(&mut self, cx: &mut Context<Self>) {
        self.ep_fallback = !self.ep_fallback;
        cx.notify();
    }

    /// Test connection with what the form says (the saved token when none is typed). Saves nothing.
    pub fn test_endpoint(&mut self, cx: &mut Context<Self>) {
        let url = self.ep_url.read(cx).text().to_string();
        let token = self.ep_token.read(cx).text().to_string();
        let id = self.ep_editing.clone();
        self.ep_busy = true;
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.test_endpoint(id.as_deref(), &url, Some(&token)).await });
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                v.ep_busy = false;
                match r {
                    Ok(Ok(t)) => {
                        v.notice = None;
                        let kind = if t.kind == EndpointKind::YmirBridge { "Ymir bridge" } else { "OpenAI-compatible" };
                        v.ep_tested = Some(format!(
                            "Test passed: {kind} · {} model{}: {}",
                            t.models.len(),
                            if t.models.len() == 1 { "" } else { "s" },
                            t.models.join(", ")
                        ));
                    }
                    Ok(Err(e)) => {
                        v.ep_tested = None;
                        v.notice = Some(e.message)
                    }
                    Err(e) => v.notice = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Saves the edit form; a changed URL or token is tested first and nothing changes on failure.
    pub fn save_endpoint(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.ep_editing.clone() else { return };
        let name = self.ep_name.read(cx).text().to_string();
        let url = self.ep_url.read(cx).text().to_string();
        let token = Some(self.ep_token.read(cx).text().to_string()).filter(|t| !t.trim().is_empty());
        self.ep_token.update(cx, |i, cx| i.set_text("", cx));
        let (model, fallback) = (self.ep_model.clone(), self.ep_fallback);
        self.ep_busy = true;
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.edit_endpoint(&id, &name, &url, token, model, fallback).await });
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                v.ep_busy = false;
                match r {
                    Ok(Ok(_)) => {
                        v.notice = None;
                        v.cancel_edit_endpoint(cx);
                    }
                    Ok(Err(e)) => v.notice = Some(e.message),
                    Err(e) => v.notice = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Delete on an endpoint: asks first, with where its sessions go.
    pub fn remove_endpoint(&mut self, id: &str, cx: &mut Context<Self>) {
        let bound = self.state.endpoints.iter().find(|e| e.id == id).map_or(0, |e| e.sessions);
        self.ep_delete = Some((id.to_string(), if bound > 0 { Some(None) } else { None }));
        cx.notify();
    }

    /// The next place in turn for the deleted endpoint's sessions: Weirdnet, each other endpoint,
    /// then nowhere (unbound).
    pub fn cycle_rebind(&mut self, cx: &mut Context<Self>) {
        let Some((id, rebind)) = self.ep_delete.clone() else { return };
        let mut opts: Vec<Option<Option<String>>> = vec![Some(None)];
        opts.extend(self.state.endpoints.iter().filter(|e| e.id != id).map(|e| Some(Some(e.id.clone()))));
        opts.push(None);
        let i = opts.iter().position(|x| *x == rebind).unwrap_or(0);
        self.ep_delete = Some((id, opts[(i + 1) % opts.len()].clone()));
        cx.notify();
    }

    pub fn confirm_delete_endpoint(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some((id, rebind)) = self.ep_delete.take()
            && yes
        {
            let r = match rebind {
                None => Rebind::Unbound,
                Some(None) => Rebind::To(Target::Weirdnet),
                Some(Some(e)) => Rebind::To(Target::Endpoint { id: e }),
            };
            let r = self.core.delete_endpoint(&id, r).map(|_| ());
            if self.try_core(r) && self.ep_editing.as_deref() == Some(id.as_str()) {
                self.cancel_edit_endpoint(cx);
            }
        }
        cx.notify();
    }

    // ---- Weirdnet links ------------------------------------------------------------------------

    /// Opens the link form on a link (`None`: a new one).
    pub fn begin_edit_link(&mut self, address: Option<&str>, cx: &mut Context<Self>) {
        let l = address.and_then(|a| self.state.links.iter().find(|l| l.address == a).cloned());
        self.link_name.update(cx, |i, cx| i.set_text(l.as_ref().map_or("", |l| l.name.as_str()), cx));
        self.link_addr.update(cx, |i, cx| i.set_text(l.as_ref().map_or("", |l| l.address.as_str()), cx));
        self.link_editing = l.map(|l| l.address);
        cx.notify();
    }

    /// Saves the link form: a new or changed address is tested first.
    pub fn save_link(&mut self, cx: &mut Context<Self>) {
        let name = self.link_name.read(cx).text().to_string();
        let addr = self.link_addr.read(cx).text().trim().to_string();
        let editing = self.link_editing.clone();
        self.link_busy = true;
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move {
            match editing {
                None => core.add_link(&name, &addr).await.map(|_| ()),
                Some(old) => {
                    let fixed = core.links().iter().any(|l| l.address == old && l.fixed);
                    let at = if fixed || addr == old { old } else { core.edit_link(&old, &addr).await?.address };
                    core.rename_link(&at, &name)
                }
            }
        });
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                v.link_busy = false;
                match r {
                    Ok(Ok(())) => {
                        v.notice = None;
                        v.begin_edit_link(None, cx);
                    }
                    Ok(Err(e)) => v.notice = Some(e.message),
                    Err(e) => v.notice = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn delete_link(&mut self, address: &str, cx: &mut Context<Self>) {
        self.link_delete = Some(address.to_string());
        cx.notify();
    }

    pub fn confirm_delete_link(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(a) = self.link_delete.take()
            && yes
        {
            let r = self.core.delete_link(&a).map(|_| ());
            if self.try_core(r) && self.link_editing.as_deref() == Some(a.as_str()) {
                self.begin_edit_link(None, cx);
            }
        }
        cx.notify();
    }

    pub fn set_fallback(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        let r = self.core.set_fallback(id);
        self.try_core(r);
        cx.notify();
    }

    // ---- drawing -----------------------------------------------------------------------------

    fn small(&self, id: impl Into<gpui::ElementId>, label: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        div()
            .id(id)
            .px(px(7.))
            .py(px(3.))
            .rounded(px(5.))
            .text_size(px(11.))
            .bg(rgb(t.surface_variant))
            .text_color(rgb(t.text_primary))
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .child(label.into())
    }

    fn target_name(&self, target: &Target) -> String {
        match target {
            Target::Weirdnet => "Weirdnet".into(),
            Target::Endpoint { id } => {
                self.state.endpoints.iter().find(|e| &e.id == id).map_or_else(|| "removed endpoint".into(), |e| e.name.clone())
            }
        }
    }

    pub(super) fn sessions_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let s = &self.state.sessions;
        let header = div()
            .id("sessions-top")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .py(px(2.))
            .rounded(px(5.))
            .when(self.moving.is_some(), |d| d.border_1().border_color(rgb(t.accent)).cursor_pointer())
            .on_click(cx.listener(|v, _, _, cx| {
                if v.moving.is_some() {
                    v.move_into(None, cx)
                }
            }))
            .drag_over::<Dragged>(move |st, _, _, _| st.bg(rgb(t.selection)))
            .on_drop(cx.listener(|v, d: &Dragged, _, cx| v.move_row(d.row.clone(), None, cx)))
            .child(div().flex_1().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(if self.moving.is_some() {
                "SESSIONS · click where it goes"
            } else {
                "SESSIONS"
            }))
            .child(self.small("new-folder", "+ Folder").on_click(cx.listener(|v, _, w, cx| {
                let parent = match &v.picked {
                    Some(Row::Folder(id)) => Some(id.clone()),
                    _ => None,
                };
                v.begin_edit(NameEdit::NewFolder { parent }, Some(w), cx)
            })))
            .child(self.small("new-session", "+ Session").on_click(cx.listener(|v, _, w, cx| {
                let folder = match &v.picked {
                    Some(Row::Folder(id)) => Some(id.clone()),
                    _ => None,
                };
                v.begin_edit(NameEdit::NewSession { folder }, Some(w), cx)
            })));

        let mut tree = div().id("sessions").flex().flex_col().gap(px(1.)).max_h(px(260.)).overflow_y_scroll();
        for (depth, row) in s.rows() {
            let indent = px(4. + 14. * depth as f32);
            let picked = self.picked.as_ref() == Some(&row);
            match &row {
                Row::Folder(id) => {
                    let Some(f) = s.folder(id) else { continue };
                    let (fid, drop_id) = (f.id.clone(), f.id.clone());
                    let moving_self = self.moving.as_ref() == Some(&row);
                    tree = tree.child(
                        div()
                            .id(SharedString::from(format!("folder-{}", f.id)))
                            .pl(indent)
                            .pr(px(6.))
                            .py(px(4.))
                            .rounded(px(5.))
                            .text_size(px(13.))
                            .text_color(rgb(t.text_primary))
                            .when(picked, |d| d.bg(rgb(t.surface_variant)))
                            .when(self.moving.is_some() && !moving_self, |d| d.border_1().border_color(rgb(t.accent)))
                            .hover(|st| st.bg(rgb(t.surface_variant)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |v, _, _, cx| v.click_folder(&fid, cx)))
                            .on_drag(Dragged { row: row.clone(), name: format!("📁 {}", f.name), theme: t }, |d, _, _, cx| cx.new(|_| d.clone()))
                            .drag_over::<Dragged>(move |st, _, _, _| st.bg(rgb(t.selection)))
                            .on_drop(cx.listener(move |v, d: &Dragged, _, cx| v.move_row(d.row.clone(), Some(drop_id.clone()), cx)))
                            .child(format!("{} 📁 {}", if f.collapsed { "▸" } else { "▾" }, f.name)),
                    );
                }
                Row::Session(id) => {
                    let Some(x) = s.get(id) else { continue };
                    let active = x.id == s.active;
                    let sid = x.id.clone();
                    let icon = if x.target == Target::Weirdnet { "◉" } else { "↗" };
                    let sub = format!("{}{}", self.target_name(&x.target), x.model.as_ref().map(|m| format!(" · {m}")).unwrap_or_default());
                    tree = tree.child(
                        div()
                            .id(SharedString::from(format!("session-{}", x.id)))
                            .flex()
                            .flex_col()
                            .pl(indent + px(12.))
                            .pr(px(6.))
                            .py(px(3.))
                            .rounded(px(5.))
                            .when(active, |d| d.bg(rgb(t.selection)))
                            .when(picked && !active, |d| d.bg(rgb(t.surface_variant)))
                            .hover(|st| st.bg(rgb(t.surface_variant)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |v, _, _, cx| v.switch_session(&sid, cx)))
                            .on_drag(Dragged { row: row.clone(), name: x.name.clone(), theme: t }, |d, _, _, cx| cx.new(|_| d.clone()))
                            .child(div().text_size(px(13.)).text_color(rgb(t.text_primary)).child(format!("{icon} {}", x.name)))
                            .child(div().text_size(px(10.)).text_color(rgb(t.text_secondary)).child(sub)),
                    );
                }
            }
        }

        // The name box, the picked row's actions, or the delete-folder question.
        let mut tools = div().flex().flex_col().gap(px(4.));
        if let Some(edit) = &self.editing {
            let mut r = div().flex().flex_row().flex_wrap().gap(px(4.));
            if matches!(edit, NameEdit::NewSession { .. }) {
                let to = self.target_name(&self.new_target.clone().map_or(Target::Weirdnet, |id| Target::Endpoint { id }));
                r = r.child(self.small("new-target", format!("Sends to {to} ▸")).on_click(cx.listener(|v, _, _, cx| v.cycle_target(cx))));
            }
            r = r
                .child(self.small("name-ok", "OK").on_click(cx.listener(|v, _, _, cx| v.commit_name(cx))))
                .child(self.small("name-cancel", "Cancel").on_click(cx.listener(|v, _, _, cx| v.cancel_edit(cx))));
            tools = tools
                .child(div().px(px(8.)).py(px(5.)).rounded(px(6.)).bg(rgb(t.background)).text_size(px(12.)).child(self.name_input.clone()))
                .child(r);
        } else if let Some(id) = &self.confirm_folder {
            let name = s.folder(id).map(|f| f.name.clone()).unwrap_or_default();
            tools = tools
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(t.text_primary))
                        .child(format!("Delete the folder {name}? What is in it moves up a level.")),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(4.))
                        .child(self.small("delete-folder-ok", "Delete folder").on_click(cx.listener(|v, _, _, cx| v.confirm_delete_folder(true, cx))))
                        .child(self.small("delete-folder-no", "Cancel").on_click(cx.listener(|v, _, _, cx| v.confirm_delete_folder(false, cx)))),
                );
        } else if self.moving.is_some() {
            tools = tools.child(self.small("move-cancel", "Cancel move").on_click(cx.listener(|v, _, _, cx| {
                v.moving = None;
                cx.notify();
            })));
        } else if let Some(row) = &self.picked {
            let mut r = div().flex().flex_row().flex_wrap().gap(px(4.));
            let (rename, deletable) = match row {
                Row::Session(id) => (NameEdit::RenameSession(id.clone()), id != DEFAULT_SESSION),
                Row::Folder(id) => (NameEdit::RenameFolder(id.clone()), true),
            };
            r = r
                .child(self.small("rename", "Rename").on_click(cx.listener(move |v, _, w, cx| v.begin_edit(rename.clone(), Some(w), cx))))
                .child(self.small("move", "Move").on_click(cx.listener(|v, _, _, cx| v.start_move(cx))));
            if let Row::Session(id) = row
                && id != DEFAULT_SESSION
                && let Some(x) = s.get(id)
            {
                r = r.child(
                    self.small("target", format!("Sends to {} ▸", self.target_name(&x.target)))
                        .on_click(cx.listener(|v, _, _, cx| v.cycle_target(cx))),
                );
            }
            if deletable {
                r = r.child(self.small("delete", "Delete").on_click(cx.listener(|v, _, _, cx| v.delete_picked(cx))));
            }
            tools = tools.child(r);
        }

        div().flex().flex_col().gap(px(4.)).child(header).child(tree).child(tools).child(
            self.button("endpoints", if self.endpoints_open { "Close endpoints" } else { "Endpoints and links" }, false)
                .on_click(cx.listener(|v, _, _, cx| v.toggle_endpoints(cx))),
        )
    }

    /// The route line under the models: where the open session's next message goes.
    pub(super) fn route_line(&self) -> impl IntoElement {
        let t = self.theme;
        let r = self.state.route();
        let color = match r.kind {
            RouteKind::Weirdnet => 0x4CD07D,
            RouteKind::Fallback => 0xFF9F43,
            RouteKind::Endpoint => t.accent,
            RouteKind::None => t.text_secondary,
        };
        let icon = match r.kind {
            RouteKind::Weirdnet => "◉",
            RouteKind::Fallback => "⤳",
            RouteKind::Endpoint => "↗",
            RouteKind::None => "○",
        };
        div()
            .id("route")
            .flex_none()
            .px(px(16.))
            .py(px(4.))
            .border_b_1()
            .border_color(rgb(t.border))
            .text_size(px(12.))
            .text_color(rgb(color))
            .child(format!("{icon} {} · {}", self.state.session().name, r.detail))
    }

    /// The Endpoints panel, drawn in place of the messages while open.
    pub(super) fn endpoints_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let mut list = div().flex().flex_col().gap(px(10.));
        if self.state.endpoints.is_empty() {
            list = list.child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(t.text_secondary))
                    .child("No endpoints yet. Add a Ymir bridge to use as the fallback, or any OpenAI-compatible API to give it a session."),
            );
        }
        for e in &self.state.endpoints {
            let (dot, health) = match e.health {
                EndpointHealth::Ok => (0x4CD07D, format!("{} model{}", e.models.len(), if e.models.len() == 1 { "" } else { "s" })),
                EndpointHealth::Failed => (t.error, format!("not answering: {}", e.message.clone().unwrap_or_default())),
                EndpointHealth::Unchecked => (t.text_secondary, "checking…".into()),
            };
            let kind = match e.kind {
                EndpointKind::YmirBridge => "Ymir bridge",
                EndpointKind::OpenAi => "OpenAI-compatible",
            };
            let (rid, eid, fid, on) = (e.id.clone(), e.id.clone(), e.id.clone(), e.fallback);
            let mut actions = div()
                .flex()
                .flex_row()
                .gap(px(6.))
                .child(
                    self.small(SharedString::from(format!("endpoint-edit-{}", e.id)), "Edit")
                        .on_click(cx.listener(move |v, _, _, cx| v.begin_edit_endpoint(&eid, cx))),
                )
                .child(
                    self.small(SharedString::from(format!("endpoint-remove-{}", e.id)), "Delete")
                        .on_click(cx.listener(move |v, _, _, cx| v.remove_endpoint(&rid, cx))),
                );
            actions = if e.kind == EndpointKind::YmirBridge {
                actions.child(
                    self.small(
                        SharedString::from(format!("endpoint-fallback-{}", e.id)),
                        if on { "✓ Fallback for Weirdnet" } else { "Use as fallback" },
                    )
                    .on_click(cx.listener(move |v, _, _, cx| v.set_fallback(if on { None } else { Some(&fid) }, cx))),
                )
            } else {
                actions.child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("Only a Ymir bridge can be the fallback"))
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("endpoint-{}", e.id)))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(rgb(t.surface))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(div().w(px(8.)).h(px(8.)).rounded(px(4.)).bg(rgb(dot)))
                            .child(div().text_size(px(14.)).child(e.name.clone()))
                            .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child(format!(
                                "{kind} · {health}{}{}",
                                e.default_model().map(|m| format!(" · default {m}")).unwrap_or_default(),
                                match e.sessions {
                                    0 => String::new(),
                                    1 => " · 1 session".into(),
                                    n => format!(" · {n} sessions"),
                                }
                            ))),
                    )
                    .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child(e.url.clone()))
                    .child(actions)
                    .children(self.delete_endpoint_question(&e.id, cx)),
            );
        }
        let field = |label: &'static str, input: gpui::Entity<crate::text_input::TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(label))
                .child(div().px(px(8.)).py(px(6.)).rounded(px(6.)).bg(rgb(t.background)).text_size(px(13.)).child(input))
        };
        let editing = self.ep_editing.as_ref().and_then(|id| self.state.endpoints.iter().find(|e| &e.id == id));
        let mut buttons = div()
            .flex()
            .flex_row()
            .gap(px(8.))
            .child(self.button("endpoint-test", "Test connection", false).on_click(cx.listener(|v, _, _, cx| v.test_endpoint(cx))));
        buttons = match editing {
            Some(_) => buttons
                .child(
                    self.button("endpoint-save", if self.ep_busy { "Testing…" } else { "Save" }, true)
                        .on_click(cx.listener(|v, _, _, cx| v.save_endpoint(cx))),
                )
                .child(self.button("endpoint-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.cancel_edit_endpoint(cx)))),
            None => buttons.child(
                self.button("endpoint-add", if self.ep_busy { "Checking…" } else { "Check and add" }, true)
                    .on_click(cx.listener(|v, _, _, cx| v.add_endpoint(cx))),
            ),
        };
        let mut form = div()
            .id("endpoint-form")
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(rgb(t.surface))
            .when(editing.is_some(), |d| d.border_1().border_color(rgb(t.accent)))
            .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(match editing {
                Some(e) => format!("EDIT {}", e.name.to_uppercase()),
                None => "ADD AN ENDPOINT".into(),
            }))
            .child(field("Name", self.ep_name.clone()))
            .child(field("URL (its OpenAI-compatible /v1)", self.ep_url.clone()))
            .child(field(if editing.is_some() { "New token (the saved one is never shown)" } else { "Token" }, self.ep_token.clone()));
        if let Some(e) = editing {
            let model = self.ep_model.clone().unwrap_or_else(|| "its first".into());
            let mut row = div().flex().flex_row().gap(px(6.)).child(
                self.small("endpoint-model", format!("Default model: {model} ▸")).on_click(cx.listener(|v, _, _, cx| v.cycle_endpoint_model(cx))),
            );
            if e.kind == EndpointKind::YmirBridge {
                row = row.child(
                    self.small("endpoint-edit-fallback", if self.ep_fallback { "✓ Fallback for Weirdnet" } else { "Not the fallback" })
                        .on_click(cx.listener(|v, _, _, cx| v.toggle_endpoint_fallback(cx))),
                );
            }
            form = form.child(row);
        }
        let form = form
            .children(self.ep_tested.clone().map(|s| div().id("endpoint-tested").text_size(px(12.)).text_color(rgb(0x4CD07D)).child(s)))
            .child(buttons)
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(t.text_secondary))
                    .child("Bragi reads its /v1/models to tell a Ymir bridge from another API. The token is kept in a private file in Bragi's data folder, never shown again."),
            );
        div()
            .id("endpoints-panel")
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(16.))
            .flex_1()
            .overflow_y_scroll()
            .child(div().text_size(px(18.)).child("Endpoints"))
            .children(self.notice.clone().map(|n| div().text_size(px(12.)).text_color(rgb(t.error)).child(n)))
            .child(list)
            .child(form)
            .child(self.links_section(cx))
    }

    /// Under an endpoint's card while it waits for "delete?": how many sessions send to it and
    /// where they will go.
    fn delete_endpoint_question(&self, id: &str, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let t = self.theme;
        let (did, rebind) = self.ep_delete.as_ref()?;
        if did != id {
            return None;
        }
        let e = self.state.endpoints.iter().find(|e| e.id == id)?;
        let mut q = div().flex().flex_col().gap(px(6.)).p(px(8.)).rounded(px(8.)).bg(rgb(t.surface_variant));
        if e.sessions == 0 {
            q = q.child(div().text_size(px(12.)).child(format!("Delete {}? No session sends to it. Its saved token goes with it.", e.name)));
        } else {
            let to = match rebind {
                None => "nowhere for now (they say so until you pick a target)".to_string(),
                Some(None) => "Weirdnet".to_string(),
                Some(Some(o)) => self.state.endpoints.iter().find(|x| &x.id == o).map_or_else(|| "?".into(), |x| x.name.clone()),
            };
            q = q
                .child(div().id("delete-endpoint-warning").text_size(px(12.)).child(format!(
                    "Delete {}? {} session{} to it. Their conversations are kept.",
                    e.name,
                    e.sessions,
                    if e.sessions == 1 { " sends" } else { "s send" }
                )))
                .child(self.small("rebind", format!("Send them to: {to} ▸")).on_click(cx.listener(|v, _, _, cx| v.cycle_rebind(cx))));
        }
        Some(
            q.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .child(self.small("delete-endpoint-ok", "Delete").on_click(cx.listener(|v, _, _, cx| v.confirm_delete_endpoint(true, cx))))
                    .child(self.small("delete-endpoint-no", "Cancel").on_click(cx.listener(|v, _, _, cx| v.confirm_delete_endpoint(false, cx)))),
            ),
        )
    }

    /// The Weirdnet links: node addresses this device dials, each renamed, edited after a test
    /// connection, or deleted after a question.
    fn links_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let mut list = div().flex().flex_col().gap(px(6.));
        for l in &self.state.links {
            let (ea, da) = (l.address.clone(), l.address.clone());
            let mut actions = div().flex().flex_row().gap(px(6.)).child(
                self.small(SharedString::from(format!("link-edit-{}", l.address)), if l.fixed { "Rename" } else { "Edit" })
                    .on_click(cx.listener(move |v, _, _, cx| v.begin_edit_link(Some(&ea), cx))),
            );
            if !l.fixed {
                actions = actions.child(
                    self.small(SharedString::from(format!("link-delete-{}", l.address)), "Delete")
                        .on_click(cx.listener(move |v, _, _, cx| v.delete_link(&da, cx))),
                );
            }
            let mut card = div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(10.))
                .rounded(px(10.))
                .bg(rgb(t.surface))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .child(div().w(px(8.)).h(px(8.)).rounded(px(4.)).bg(rgb(if l.connected { 0x4CD07D } else { t.text_secondary })))
                        .child(div().text_size(px(14.)).child(l.name.clone()))
                        .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(if l.connected {
                            "connected"
                        } else {
                            "not connected"
                        })),
                )
                .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(l.address.clone()))
                .child(actions);
            if self.link_delete.as_deref() == Some(l.address.as_str()) {
                card = card.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .p(px(8.))
                        .rounded(px(8.))
                        .bg(rgb(t.surface_variant))
                        .child(div().text_size(px(12.)).child(format!(
                            "Delete the link {}? Bragi forgets the address and stops dialing it. No session is bound to it and no conversation is deleted.",
                            l.name
                        )))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(6.))
                                .child(self.small("delete-link-ok", "Delete link").on_click(cx.listener(|v, _, _, cx| v.confirm_delete_link(true, cx))))
                                .child(self.small("delete-link-no", "Cancel").on_click(cx.listener(|v, _, _, cx| v.confirm_delete_link(false, cx)))),
                        ),
                );
            }
            list = list.child(card);
        }
        let field = |label: &'static str, input: gpui::Entity<crate::text_input::TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(label))
                .child(div().px(px(8.)).py(px(6.)).rounded(px(6.)).bg(rgb(t.background)).text_size(px(13.)).child(input))
        };
        let editing = self.link_editing.as_ref().and_then(|a| self.state.links.iter().find(|l| &l.address == a));
        let mut buttons = div().flex().flex_row().gap(px(8.)).child(
            self.button(
                "link-save",
                if self.link_busy {
                    "Testing…"
                } else if editing.is_some() {
                    "Test and save"
                } else {
                    "Test and add"
                },
                true,
            )
            .on_click(cx.listener(|v, _, _, cx| v.save_link(cx))),
        );
        if editing.is_some() {
            buttons = buttons.child(self.button("link-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.begin_edit_link(None, cx))));
        }
        let mut form = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(rgb(t.surface))
            .when(editing.is_some(), |d| d.border_1().border_color(rgb(t.accent)))
            .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(match editing {
                Some(l) => format!("EDIT {}", l.name.to_uppercase()),
                None => "ADD A LINK".into(),
            }))
            .child(field("Name", self.link_name.clone()));
        if !editing.is_some_and(|l| l.fixed) {
            form = form.child(field("Node address (…/p2p/PEER)", self.link_addr.clone()));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(18.)).child("Weirdnet links"))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(t.text_secondary))
                    .child("Node addresses this device dials. Sessions use the whole pool, so no session depends on one link."),
            )
            .child(list)
            .child(form.child(buttons))
    }
}
