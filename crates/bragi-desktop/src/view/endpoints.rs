//! Endpoints, links, sessions and folders in the window (LF-11): the Settings sections that add,
//! edit, delete, test and reorder OpenAI-compatible endpoints and Weirdnet links, the delete confirm
//! with its rebind choice, the folder tree in the sidebar, and the target picker. Every change is
//! one `Core` call; this only asks, shows and refreshes, the same flow as the Flutter app.

use std::collections::HashSet;

use bragi_core::{EndpointInput, EndpointMode, EndpointView, Folder, KeyChange, LinkView, SessionSummary, Target};

use super::*;

/// What the form card edits.
#[derive(Clone)]
pub(super) enum Form {
    Endpoint { id: Option<String>, mode: EndpointMode, enabled: bool, has_key: bool, clear_key: bool },
    Link { old: Option<String> },
    Folder { id: Option<String>, parent: Option<String> },
}

/// An endpoint delete waiting for its confirm, with where its chats go.
pub(super) struct DeleteEndpoint {
    pub id: String,
    pub name: String,
    pub chats: usize,
    pub options: Vec<bragi_core::TargetOption>,
    /// Index into `options`, or `None` for "leave without a target".
    pub pick: Option<usize>,
}

/// Settings state: the lists as the core last said them, and whatever card is open.
#[derive(Default)]
pub(super) struct Prefs {
    pub endpoints: Vec<EndpointView>,
    pub links: Vec<LinkView>,
    pub targets: Vec<bragi_core::TargetOption>,
    pub folders: Vec<Folder>,
    pub sessions: Vec<SessionSummary>,
    pub form: Option<Form>,
    pub form_error: Option<String>,
    pub delete_endpoint: Option<DeleteEndpoint>,
    pub delete_link: Option<String>,
    pub delete_folder: Option<(String, String)>,
    /// Test results by endpoint id (and `form` for the card being edited): ok, words.
    pub tests: HashMap<String, (bool, String)>,
    /// Folders the user collapsed.
    pub collapsed: HashSet<String>,
    /// Where the next new chat goes; `None` is the default.
    pub new_target: Option<Target>,
    pub target_menu: bool,
    /// The conversation whose folder is being picked.
    pub move_menu: Option<String>,
}

/// The form's text fields, reused by every card.
pub(super) struct Fields {
    pub name: Entity<TextInput>,
    pub url: Entity<TextInput>,
    pub key: Entity<TextInput>,
    pub model: Entity<TextInput>,
}

impl Fields {
    pub fn new(theme: Theme, cx: &mut Context<BragiView>) -> Self {
        Self {
            name: cx.new(|cx| TextInput::new("Name", theme, cx)),
            url: cx.new(|cx| TextInput::new("https://host/v1", theme, cx)),
            key: cx.new(|cx| TextInput::new("API key (optional)", theme, cx).masked()),
            model: cx.new(|cx| TextInput::new("Default model (optional)", theme, cx)),
        }
    }
}

fn mode_name(m: EndpointMode) -> &'static str {
    match m {
        EndpointMode::Fallback => "Fallback",
        EndpointMode::Alongside => "Alongside the pool",
    }
}

/// A node address shortened for a row: the host and the end of the peer id.
pub(super) fn short_address(a: &str) -> String {
    let host = a.split('/').nth(2).unwrap_or(a);
    let peer = a.rsplit("/p2p/").next().unwrap_or_default();
    if peer.len() > 8 && peer != a { format!("{host} · …{}", &peer[peer.len() - 6..]) } else { a.to_string() }
}

impl BragiView {
    /// Re-reads everything Settings and the sidebar show.
    pub(super) fn refresh_prefs(&mut self) {
        let p = &mut self.prefs;
        p.endpoints = self.core.endpoints();
        p.links = self.core.links();
        p.targets = self.core.targets();
        p.folders = self.core.folders();
        p.sessions = self.core.sessions(0, 100).unwrap_or_default();
        if let Some(t) = &p.new_target
            && !p.targets.iter().any(|o| &o.target == t)
        {
            p.new_target = None;
        }
    }

    fn fail(&mut self, e: bragi_core::CoreError, cx: &mut Context<Self>) {
        self.prefs.form_error = Some(e.message);
        cx.notify();
    }

    fn text(&self, f: &Entity<TextInput>, cx: &Context<Self>) -> String {
        f.read(cx).text().trim().to_string()
    }

    fn set(&self, f: &Entity<TextInput>, text: &str, cx: &mut Context<Self>) {
        f.update(cx, |i, cx| i.set_text(text, cx));
    }

    // ---- endpoints ------------------------------------------------------------------------

    pub fn endpoint_form(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        let e = id.and_then(|id| self.prefs.endpoints.iter().find(|e| e.id == id)).cloned();
        let f = &self.fields;
        let (name, url, model) = e.as_ref().map(|e| (e.name.clone(), e.base_url.clone(), e.model.clone().unwrap_or_default())).unwrap_or_default();
        let has_key = e.as_ref().is_some_and(|e| e.has_key);
        let (n, u, m, k) = (f.name.clone(), f.url.clone(), f.model.clone(), f.key.clone());
        self.set(&n, &name, cx);
        self.set(&u, &url, cx);
        self.set(&m, &model, cx);
        self.set(&k, "", cx);
        k.update(cx, |i, cx| i.configure(if has_key { "Leave blank to keep the key" } else { "API key (optional)" }, true, cx));
        self.prefs.tests.remove("form");
        self.prefs.form_error = None;
        self.prefs.form = Some(Form::Endpoint {
            id: e.as_ref().map(|e| e.id.clone()),
            mode: e.as_ref().map(|e| e.mode).unwrap_or_default(),
            enabled: e.as_ref().is_none_or(|e| e.enabled),
            has_key,
            clear_key: false,
        });
        cx.notify();
    }

    pub fn form_mode(&mut self, m: EndpointMode, cx: &mut Context<Self>) {
        if let Some(Form::Endpoint { mode, .. }) = &mut self.prefs.form {
            *mode = m;
        }
        cx.notify();
    }

    pub fn form_enabled(&mut self, cx: &mut Context<Self>) {
        if let Some(Form::Endpoint { enabled, .. }) = &mut self.prefs.form {
            *enabled = !*enabled;
        }
        cx.notify();
    }

    pub fn form_clear_key(&mut self, cx: &mut Context<Self>) {
        if let Some(Form::Endpoint { clear_key, .. }) = &mut self.prefs.form {
            *clear_key = !*clear_key;
        }
        cx.notify();
    }

    pub fn close_form(&mut self, cx: &mut Context<Self>) {
        self.prefs.form = None;
        self.prefs.form_error = None;
        cx.notify();
    }

    /// Save on the open card: an endpoint, a link or a folder.
    pub fn form_save(&mut self, cx: &mut Context<Self>) {
        self.prefs.form_error = None;
        match self.prefs.form.clone() {
            Some(Form::Endpoint { id, mode, enabled, clear_key, .. }) => {
                let input = EndpointInput {
                    name: self.text(&self.fields.name, cx),
                    base_url: self.text(&self.fields.url, cx),
                    model: Some(self.text(&self.fields.model, cx)).filter(|m| !m.is_empty()),
                    mode,
                    enabled,
                };
                let key = self.text(&self.fields.key, cx);
                let done = match id {
                    None => self.core.add_endpoint(input, Some(key).filter(|k| !k.is_empty())).map(|_| ()),
                    Some(id) => {
                        let change = if clear_key {
                            KeyChange::Clear
                        } else if key.is_empty() {
                            KeyChange::Keep
                        } else {
                            KeyChange::Set(key)
                        };
                        self.core.edit_endpoint(&id, input, change).map(|_| ())
                    }
                };
                match done {
                    Ok(()) => self.close_form(cx),
                    Err(e) => return self.fail(e, cx),
                }
            }
            Some(Form::Link { old }) => {
                let address = self.text(&self.fields.url, cx);
                let done = match &old {
                    None => self.core.add_node(&address).map(|_| ()),
                    Some(old) => self.core.edit_link(old, &address).map(|_| ()),
                };
                match done {
                    Ok(()) => self.close_form(cx),
                    Err(e) => return self.fail(e, cx),
                }
            }
            Some(Form::Folder { id, parent }) => {
                let name = self.text(&self.fields.name, cx);
                let done = match &id {
                    None => self.core.create_folder(&name, parent.as_deref()).map(|_| ()),
                    Some(id) => self.core.rename_folder(id, &name),
                };
                match done {
                    Ok(()) => self.close_form(cx),
                    Err(e) => return self.fail(e, cx),
                }
            }
            None => return,
        }
        self.refresh_prefs();
        cx.notify();
    }

    /// Tests the endpoint on the card (its typed address and key) or a saved one, and says how it went.
    pub fn endpoint_test(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let slot = id.clone().unwrap_or_else(|| "form".into());
        let core = self.core.clone();
        let form_id = match &self.prefs.form {
            Some(Form::Endpoint { id, .. }) => id.clone(),
            _ => None,
        };
        let (url, key) = (self.text(&self.fields.url, cx), self.text(&self.fields.key, cx));
        let job = self.core.handle().spawn(async move {
            match (&id, &form_id, key.is_empty()) {
                (Some(id), _, _) => core.test_endpoint(id).await,
                (None, Some(id), true) => core.test_endpoint(id).await,
                _ => core.test_endpoint_address(&url, Some(key).filter(|k| !k.is_empty())).await,
            }
        });
        self.prefs.tests.insert(slot.clone(), (true, "Testing…".into()));
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                let line = match r {
                    Ok(Ok(t)) if t.ok => (true, format!("{} · {} ms", t.message, t.latency_ms)),
                    Ok(Ok(t)) => (false, t.message),
                    Ok(Err(e)) => (false, e.message),
                    Err(e) => (false, e.to_string()),
                };
                v.prefs.tests.insert(slot, line);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Moves an endpoint one place up (`-1`) or down (`1`) in the fallback order.
    pub fn endpoint_move(&mut self, id: &str, by: i32, cx: &mut Context<Self>) {
        if let Some(i) = self.prefs.endpoints.iter().position(|e| e.id == id) {
            let to = (i as i32 + by).clamp(0, self.prefs.endpoints.len() as i32 - 1) as usize;
            if let Err(e) = self.core.move_endpoint(id, to) {
                self.notice = Some(e.message);
            }
            self.refresh_prefs();
        }
        cx.notify();
    }

    pub fn ask_delete_endpoint(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(e) = self.prefs.endpoints.iter().find(|e| e.id == id) else { return };
        let options: Vec<_> =
            self.prefs.targets.iter().filter(|o| !matches!(&o.target, Target::Endpoint { id: t, .. } if t == id)).cloned().collect();
        let chats = self.core.endpoint_sessions(id);
        self.prefs.delete_endpoint =
            Some(DeleteEndpoint { id: id.to_string(), name: e.name.clone(), chats, pick: (!options.is_empty()).then_some(0), options });
        cx.notify();
    }

    pub fn pick_rebind(&mut self, pick: Option<usize>, cx: &mut Context<Self>) {
        if let Some(d) = &mut self.prefs.delete_endpoint {
            d.pick = pick;
        }
        cx.notify();
    }

    pub fn confirm_delete_endpoint(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.prefs.delete_endpoint.take() else { return };
        let to = if d.chats > 0 { d.pick.and_then(|i| d.options.get(i)).map(|o| o.target.clone()) } else { None };
        self.notice = match self.core.delete_endpoint(&d.id, to.clone()) {
            Ok(r) if r.rebound > 0 => Some(format!("Deleted {}; {} chat{} moved.", d.name, r.rebound, if r.rebound == 1 { "" } else { "s" })),
            Ok(r) if r.unbound > 0 => {
                Some(format!("Deleted {}; {} chat{} need a new target. History is kept.", d.name, r.unbound, if r.unbound == 1 { "" } else { "s" }))
            }
            Ok(_) => None,
            Err(e) => Some(e.message),
        };
        self.prefs.tests.remove(&d.id);
        self.reload();
        cx.notify();
    }

    // ---- links ----------------------------------------------------------------------------

    pub fn link_form(&mut self, old: Option<&str>, cx: &mut Context<Self>) {
        let u = self.fields.url.clone();
        self.set(&u, old.unwrap_or(""), cx);
        u.update(cx, |i, cx| i.configure("/ip4/…/tcp/4848/p2p/…", false, cx));
        self.prefs.form_error = None;
        self.prefs.form = Some(Form::Link { old: old.map(str::to_string) });
        cx.notify();
    }

    pub fn ask_delete_link(&mut self, address: &str, cx: &mut Context<Self>) {
        self.prefs.delete_link = Some(address.to_string());
        cx.notify();
    }

    pub fn confirm_delete_link(&mut self, cx: &mut Context<Self>) {
        if let Some(a) = self.prefs.delete_link.take()
            && let Err(e) = self.core.remove_link(&a)
        {
            self.notice = Some(e.message);
        }
        self.refresh_prefs();
        cx.notify();
    }

    // ---- sessions, folders and targets ------------------------------------------------------

    pub fn folder_form(&mut self, id: Option<&str>, parent: Option<&str>, cx: &mut Context<Self>) {
        let name = id.and_then(|id| self.prefs.folders.iter().find(|f| f.id == id)).map(|f| f.name.clone()).unwrap_or_default();
        let n = self.fields.name.clone();
        self.set(&n, &name, cx);
        self.prefs.form_error = None;
        self.prefs.form = Some(Form::Folder { id: id.map(str::to_string), parent: parent.map(str::to_string) });
        cx.notify();
    }

    pub fn toggle_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.prefs.collapsed.remove(id) {
            self.prefs.collapsed.insert(id.to_string());
        }
        cx.notify();
    }

    pub fn ask_delete_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(f) = self.prefs.folders.iter().find(|f| f.id == id) {
            self.prefs.delete_folder = Some((f.id.clone(), f.name.clone()));
        }
        cx.notify();
    }

    pub fn confirm_delete_folder(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _)) = self.prefs.delete_folder.take()
            && let Err(e) = self.core.delete_folder(&id)
        {
            self.notice = Some(e.message);
        }
        self.refresh_prefs();
        cx.notify();
    }

    pub fn ask_move(&mut self, conversation: &str, cx: &mut Context<Self>) {
        self.prefs.move_menu = Some(conversation.to_string());
        cx.notify();
    }

    /// Files the conversation being moved under `folder` (`None`: the top level).
    pub fn move_to(&mut self, folder: Option<String>, cx: &mut Context<Self>) {
        if let Some(c) = self.prefs.move_menu.take()
            && let Err(e) = self.core.set_session_folder(&c, folder.as_deref())
        {
            self.notice = Some(e.message);
        }
        self.refresh_prefs();
        cx.notify();
    }

    pub fn toggle_target_menu(&mut self, cx: &mut Context<Self>) {
        self.refresh_prefs();
        self.prefs.target_menu = !self.prefs.target_menu;
        cx.notify();
    }

    /// Picks where this chat goes: binds the open chat, or sets where the next new chat starts.
    pub fn pick_target(&mut self, target: Target, cx: &mut Context<Self>) {
        self.prefs.target_menu = false;
        match self.current.as_ref().map(|c| c.id.clone()) {
            Some(id) => match self.core.bind_session(&id, target) {
                Ok(()) => self.notice = None,
                Err(e) => self.notice = Some(e.message),
            },
            None => self.prefs.new_target = Some(target),
        }
        self.refresh_prefs();
        cx.notify();
    }

    /// The words for where this chat goes, and whether it needs a pick.
    pub(super) fn target_line(&self) -> (String, bool) {
        match &self.current {
            Some(c) => match self.prefs.sessions.iter().find(|s| s.id == c.id) {
                Some(s) => (s.target_label.clone(), s.unbound),
                None => ("Weirdnet pool".into(), false),
            },
            None => {
                let t = self.prefs.new_target.clone().unwrap_or(Target::Pool);
                (self.prefs.targets.iter().find(|o| o.target == t).map(|o| o.label.clone()).unwrap_or_else(|| "Weirdnet pool".into()), false)
            }
        }
    }

    // ---- drawing --------------------------------------------------------------------------

    fn small(&self, id: impl Into<gpui::ElementId>, label: &str, danger: bool) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        div()
            .id(id)
            .px(px(8.))
            .py(px(3.))
            .rounded(px(6.))
            .text_size(px(12.))
            .bg(rgb(t.surface_variant))
            .text_color(rgb(if danger { t.error } else { t.text_primary }))
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .child(label.to_string())
    }

    fn heading(&self, s: &str) -> gpui::Div {
        div().text_size(px(11.)).text_color(rgb(self.theme.text_secondary)).child(s.to_string())
    }

    /// The Settings sections for endpoints and links (history follows them).
    pub(super) fn endpoints_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let mut list = div().flex().flex_col().gap(px(6.));
        if self.prefs.endpoints.is_empty() {
            list = list.child(
                div()
                    .id("no-endpoints")
                    .text_size(px(12.))
                    .text_color(rgb(t.text_secondary))
                    .child("No endpoints yet. Any server that speaks the OpenAI chat API works."),
            );
        }
        let last = self.prefs.endpoints.len().saturating_sub(1);
        for (i, e) in self.prefs.endpoints.iter().enumerate() {
            let (id1, id2, id3, id4, id5) = (e.id.clone(), e.id.clone(), e.id.clone(), e.id.clone(), e.id.clone());
            let mut sub = format!("{} · {}", e.base_url, mode_name(e.mode));
            if !e.enabled {
                sub.push_str(" · off");
            }
            if e.has_key {
                sub.push_str(" · key set");
            }
            let test = self.prefs.tests.get(&e.id).cloned();
            list = list.child(
                div()
                    .id(SharedString::from(format!("endpoint-{}", e.id)))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(t.surface))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .child(div().text_size(px(14.)).child(e.name.clone()))
                                    .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).overflow_hidden().child(sub)),
                            )
                            .child(
                                self.small(SharedString::from(format!("endpoint-up-{}", e.id)), "↑", false)
                                    .when(i == 0, |d| d.opacity(0.4))
                                    .on_click(cx.listener(move |v, _, _, cx| v.endpoint_move(&id1, -1, cx))),
                            )
                            .child(
                                self.small(SharedString::from(format!("endpoint-down-{}", e.id)), "↓", false)
                                    .when(i == last, |d| d.opacity(0.4))
                                    .on_click(cx.listener(move |v, _, _, cx| v.endpoint_move(&id2, 1, cx))),
                            )
                            .child(
                                self.small(SharedString::from(format!("endpoint-test-{}", e.id)), "Test", false)
                                    .on_click(cx.listener(move |v, _, _, cx| v.endpoint_test(Some(id3.clone()), cx))),
                            )
                            .child(
                                self.small(SharedString::from(format!("endpoint-edit-{}", e.id)), "Edit", false)
                                    .on_click(cx.listener(move |v, _, _, cx| v.endpoint_form(Some(&id4), cx))),
                            )
                            .child(
                                self.small(SharedString::from(format!("endpoint-delete-{}", e.id)), "Delete…", true)
                                    .on_click(cx.listener(move |v, _, _, cx| v.ask_delete_endpoint(&id5, cx))),
                            ),
                    )
                    .children(test.map(|(ok, line)| div().text_size(px(12.)).text_color(rgb(if ok { t.success } else { t.error })).child(line))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(self.heading("OPENAI-COMPATIBLE ENDPOINTS"))
            .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child("Fallback endpoints answer only when the Weirdnet pool can't, in this order. Alongside ones can be picked for a chat next to the pool. Keys are kept in the OS keychain when there is one, and are never synced."))
            .child(list)
            .child(self.button("add-endpoint", "Add an endpoint", false).w(px(160.)).on_click(cx.listener(|v, _, _, cx| v.endpoint_form(None, cx))))
    }

    pub(super) fn links_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let mut list = div().flex().flex_col().gap(px(6.));
        if self.prefs.links.is_empty() {
            list = list.child(div().id("no-links").text_size(px(12.)).text_color(rgb(t.text_secondary)).child("No node addresses added."));
        }
        for (i, l) in self.prefs.links.iter().enumerate() {
            let (a1, a2) = (l.address.clone(), l.address.clone());
            list = list.child(
                div()
                    .id(SharedString::from(format!("link-{i}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(t.surface))
                    .child(div().text_size(px(12.)).text_color(rgb(if l.online { t.success } else { t.text_secondary })).child(if l.online {
                        "●"
                    } else {
                        "○"
                    }))
                    .child(div().flex_1().min_w(px(0.)).overflow_hidden().text_size(px(13.)).child(short_address(&l.address)))
                    .child(
                        self.small(SharedString::from(format!("link-edit-{i}")), "Edit", false)
                            .on_click(cx.listener(move |v, _, _, cx| v.link_form(Some(&a1), cx))),
                    )
                    .child(
                        self.small(SharedString::from(format!("link-delete-{i}")), "Delete…", true)
                            .on_click(cx.listener(move |v, _, _, cx| v.ask_delete_link(&a2, cx))),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(self.heading("WEIRDNET LINKS"))
            .child(list)
            .child(self.button("add-link", "Add a link", false).w(px(120.)).on_click(cx.listener(|v, _, _, cx| v.link_form(None, cx))))
    }

    fn modal(&self, id: &'static str, card: gpui::Div) -> gpui::Stateful<gpui::Div> {
        div().id(id).absolute().top_0().left_0().size_full().flex().items_center().justify_center().bg(gpui::rgba(0x000000aa)).occlude().child(card)
    }

    fn card(&self, title: &str) -> gpui::Div {
        let t = self.theme;
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .w(px(460.))
            .max_h(px(640.))
            .p(px(20.))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.surface))
            .child(div().text_size(px(16.)).child(title.to_string()))
    }

    fn field(&self, input: &Entity<TextInput>) -> gpui::Div {
        div().px(px(10.)).py(px(8.)).rounded(px(6.)).bg(rgb(self.theme.background)).text_size(px(13.)).child(input.clone())
    }

    /// The overlays: the form, the delete confirms, the folder and target pickers.
    pub(super) fn prefs_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let t = self.theme;
        if let Some(form) = self.prefs.form.clone() {
            let err = self.prefs.form_error.clone().map(|e| div().id("form-error").text_size(px(12.)).text_color(rgb(t.error)).child(e));
            let actions = |extra: Option<gpui::AnyElement>| {
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .justify_end()
                    .children(extra)
                    .child(self.button("form-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.close_form(cx))))
                    .child(self.button("form-save", "Save", true).on_click(cx.listener(|v, _, _, cx| v.form_save(cx))))
            };
            let card = match form {
                Form::Endpoint { id, mode, enabled, has_key, clear_key } => {
                    let test = self.prefs.tests.get("form").cloned();
                    let toggle = |id: &'static str, label: &str, on: bool| self.button(id, label.to_string(), on);
                    let mut c = self
                        .card(if id.is_some() { "Edit endpoint" } else { "Add an endpoint" })
                        .child(self.field(&self.fields.name))
                        .child(self.field(&self.fields.url))
                        .child(self.field(&self.fields.key))
                        .child(self.field(&self.fields.model))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.))
                                .child(
                                    toggle("mode-fallback", "Fallback", mode == EndpointMode::Fallback)
                                        .on_click(cx.listener(|v, _, _, cx| v.form_mode(EndpointMode::Fallback, cx))),
                                )
                                .child(
                                    toggle("mode-alongside", "Alongside the pool", mode == EndpointMode::Alongside)
                                        .on_click(cx.listener(|v, _, _, cx| v.form_mode(EndpointMode::Alongside, cx))),
                                )
                                .child(
                                    toggle("form-enabled", if enabled { "On" } else { "Off" }, enabled)
                                        .on_click(cx.listener(|v, _, _, cx| v.form_enabled(cx))),
                                ),
                        );
                    if has_key {
                        c = c.child(
                            self.button("form-clear-key", if clear_key { "Key will be removed" } else { "Remove the saved key" }, clear_key)
                                .on_click(cx.listener(|v, _, _, cx| v.form_clear_key(cx))),
                        );
                    }
                    c = c.children(
                        test.map(|(ok, l)| div().id("form-test").text_size(px(12.)).text_color(rgb(if ok { t.success } else { t.error })).child(l)),
                    );
                    let test_btn =
                        self.button("form-test-run", "Test", false).on_click(cx.listener(|v, _, _, cx| v.endpoint_test(None, cx))).into_any_element();
                    c.children(err).child(actions(Some(test_btn)))
                }
                Form::Link { old } => self
                    .card(if old.is_some() { "Edit link" } else { "Add a link" })
                    .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child("A node address from `ymirctl pool addresses`."))
                    .child(self.field(&self.fields.url))
                    .children(err)
                    .child(actions(None)),
                Form::Folder { id, .. } => self
                    .card(if id.is_some() { "Rename folder" } else { "New folder" })
                    .child(self.field(&self.fields.name))
                    .children(err)
                    .child(actions(None)),
            };
            return Some(self.modal("form", card).into_any_element());
        }
        if let Some(d) = &self.prefs.delete_endpoint {
            let mut c = self.card(&format!("Delete {}?", d.name)).child(
                div()
                    .id("ep-delete-text")
                    .text_size(px(13.))
                    .text_color(rgb(t.text_secondary))
                    .child("Its address and API key are removed from this device. Your chats and their history are kept."),
            );
            if d.chats > 0 {
                c = c.child(div().id("ep-delete-bound").text_size(px(13.)).child(format!(
                    "{} chat{} bound to it. Where should {} go?",
                    d.chats,
                    if d.chats == 1 { " is" } else { "s are" },
                    if d.chats == 1 { "it" } else { "they" }
                )));
                for (i, o) in d.options.iter().enumerate() {
                    c = c.child(
                        self.radio(SharedString::from(format!("rebind-{i}")), &o.label, d.pick == Some(i))
                            .on_click(cx.listener(move |v, _, _, cx| v.pick_rebind(Some(i), cx))),
                    );
                }
                c = c.child(
                    self.radio("rebind-unbound".into(), "Leave without a target", d.pick.is_none())
                        .on_click(cx.listener(|v, _, _, cx| v.pick_rebind(None, cx))),
                );
            }
            let c = c.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .justify_end()
                    .child(self.button("ep-delete-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| {
                        v.prefs.delete_endpoint = None;
                        cx.notify();
                    })))
                    .child(
                        self.button("ep-delete-confirm", "Delete", true)
                            .bg(rgb(t.error))
                            .text_color(rgb(t.background))
                            .on_click(cx.listener(|v, _, _, cx| v.confirm_delete_endpoint(cx))),
                    ),
            );
            return Some(self.modal("delete-endpoint", c).into_any_element());
        }
        if let Some(a) = &self.prefs.delete_link {
            let c = self
                .card("Delete this link?")
                .child(
                    div()
                        .id("link-delete-text")
                        .text_size(px(13.))
                        .text_color(rgb(t.text_secondary))
                        .child(format!("Bragi stops dialing {}. Your chats and history are kept.", short_address(a))),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .justify_end()
                        .child(self.button("link-delete-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| {
                            v.prefs.delete_link = None;
                            cx.notify();
                        })))
                        .child(
                            self.button("link-delete-confirm", "Delete", true)
                                .bg(rgb(t.error))
                                .text_color(rgb(t.background))
                                .on_click(cx.listener(|v, _, _, cx| v.confirm_delete_link(cx))),
                        ),
                );
            return Some(self.modal("delete-link", c).into_any_element());
        }
        if let Some((_, name)) = &self.prefs.delete_folder {
            let c = self
                .card(&format!("Delete the folder “{name}”?"))
                .child(
                    div()
                        .id("folder-delete-text")
                        .text_size(px(13.))
                        .text_color(rgb(t.text_secondary))
                        .child("Its chats and folders move up one level. No chat is deleted."),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .justify_end()
                        .child(self.button("folder-delete-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| {
                            v.prefs.delete_folder = None;
                            cx.notify();
                        })))
                        .child(
                            self.button("folder-delete-confirm", "Delete", true)
                                .bg(rgb(t.error))
                                .text_color(rgb(t.background))
                                .on_click(cx.listener(|v, _, _, cx| v.confirm_delete_folder(cx))),
                        ),
                );
            return Some(self.modal("delete-folder", c).into_any_element());
        }
        if self.prefs.move_menu.is_some() {
            let mut c = self
                .card("Move to folder")
                .child(self.radio("pick-top".into(), "Top level", false).on_click(cx.listener(|v, _, _, cx| v.move_to(None, cx))));
            for (f, depth) in self.folder_order() {
                let id = f.id.clone();
                c = c.child(
                    self.radio(SharedString::from(format!("pick-{}", f.id)), &format!("{}{}", "    ".repeat(depth), f.name), false)
                        .on_click(cx.listener(move |v, _, _, cx| v.move_to(Some(id.clone()), cx))),
                );
            }
            let c = c.child(self.button("move-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| {
                v.prefs.move_menu = None;
                cx.notify();
            })));
            return Some(self.modal("move", c).into_any_element());
        }
        if self.prefs.target_menu {
            let (_, unbound) = self.target_line();
            let mut c = self.card("Where this chat goes");
            if unbound {
                c = c.child(div().text_size(px(12.)).text_color(rgb(t.warning)).child("Its endpoint was deleted. Pick where to send this chat."));
            }
            for (i, o) in self.prefs.targets.iter().enumerate() {
                let target = o.target.clone();
                c = c.child(
                    self.radio(SharedString::from(format!("target-{i}")), &o.label, false)
                        .on_click(cx.listener(move |v, _, _, cx| v.pick_target(target.clone(), cx))),
                );
            }
            let c = c.child(self.button("target-close", "Close", false).on_click(cx.listener(|v, _, _, cx| v.toggle_target_menu(cx))));
            return Some(self.modal("targets", c).into_any_element());
        }
        None
    }

    fn radio(&self, id: SharedString, label: &str, on: bool) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        div()
            .id(id)
            .flex()
            .flex_row()
            .gap(px(8.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(rgb(if on { t.surface_variant } else { t.surface }))
            .hover(|s| s.bg(rgb(t.surface_variant)))
            .cursor_pointer()
            .text_size(px(13.))
            .child(if on { "◉" } else { "○" })
            .child(label.to_string())
    }

    /// Folders depth-first, each with its depth, parents before children.
    pub(super) fn folder_order(&self) -> Vec<(Folder, usize)> {
        fn walk(all: &[Folder], parent: Option<&str>, depth: usize, out: &mut Vec<(Folder, usize)>) {
            for f in all.iter().filter(|f| f.parent.as_deref() == parent) {
                out.push((f.clone(), depth));
                walk(all, Some(&f.id), depth + 1, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.prefs.folders, None, 0, &mut out);
        out
    }

    fn session_row(&self, s: &SessionSummary, depth: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let selected = self.current.as_ref().is_some_and(|x| x.id == s.id);
        let (id, mv) = (s.id.clone(), s.id.clone());
        let label = (s.unbound || s.target_label != "Weirdnet pool")
            .then(|| div().text_size(px(10.)).text_color(rgb(if s.unbound { t.warning } else { t.text_secondary })).child(s.target_label.clone()));
        div()
            .id(SharedString::from(format!("conv-{}", s.id)))
            .pl(px(10. + 14. * depth as f32))
            .pr(px(10.))
            .py(px(7.))
            .rounded(px(6.))
            .text_size(px(13.))
            .text_color(rgb(if selected { t.text_primary } else { t.text_secondary }))
            .when(selected, |d| d.bg(rgb(t.surface_variant)))
            .hover(|s| s.bg(rgb(t.surface_variant)))
            .cursor_pointer()
            .on_click(cx.listener(move |v, _, _, cx| v.open(&id, cx)))
            .flex()
            .flex_row()
            .items_center()
            .child(div().flex_1().min_w(px(0.)).overflow_hidden().flex().flex_col().child(s.title.clone()).children(label))
            .child(
                div()
                    .id(SharedString::from(format!("move-conv-{}", s.id)))
                    .flex_none()
                    .px(px(4.))
                    .text_size(px(12.))
                    .text_color(rgb(t.text_secondary))
                    .hover(|s| s.text_color(rgb(t.accent)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |v, _, _, cx| {
                        cx.stop_propagation();
                        v.ask_move(&mv, cx)
                    }))
                    .child("↪"),
            )
            .child(self.conversation_delete(&s.id, cx))
            .into_any_element()
    }

    /// The history tree: folders (nested, collapsible) and the chats in them.
    pub(super) fn history_tree(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let t = self.theme;
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut shown_under: HashSet<String> = HashSet::new();
        let known: HashSet<&str> = self.prefs.folders.iter().map(|f| f.id.as_str()).collect();
        let mut hidden: HashSet<String> = HashSet::new();
        for (f, depth) in self.folder_order() {
            // Under a collapsed ancestor: not drawn.
            if f.parent.as_ref().is_some_and(|p| hidden.contains(p) || self.prefs.collapsed.contains(p)) {
                hidden.insert(f.id.clone());
                continue;
            }
            let open = !self.prefs.collapsed.contains(&f.id);
            let (a, b, c, d) = (f.id.clone(), f.id.clone(), f.id.clone(), f.id.clone());
            rows.push(
                div()
                    .id(SharedString::from(format!("folder-{}", f.id)))
                    .pl(px(6. + 14. * depth as f32))
                    .pr(px(6.))
                    .py(px(5.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(rgb(t.text_primary))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(t.surface_variant)))
                    .rounded(px(6.))
                    .on_click(cx.listener(move |v, _, _, cx| v.toggle_folder(&a, cx)))
                    .child(if open { "▾" } else { "▸" })
                    .child(div().flex_1().min_w(px(0.)).overflow_hidden().child(f.name.clone()))
                    .child(self.folder_btn(format!("folder-sub-{}", f.id), "+", cx, move |v, cx| v.folder_form(None, Some(&b), cx)))
                    .child(self.folder_btn(format!("folder-rename-{}", f.id), "✎", cx, move |v, cx| v.folder_form(Some(&c), None, cx)))
                    .child(self.folder_btn(format!("folder-delete-{}", f.id), "×", cx, move |v, cx| v.ask_delete_folder(&d, cx)))
                    .into_any_element(),
            );
            if open {
                shown_under.insert(f.id.clone());
                for s in self.prefs.sessions.iter().filter(|s| s.folder.as_deref() == Some(f.id.as_str())) {
                    rows.push(self.session_row(s, depth + 1, cx));
                }
            }
        }
        // Chats at the top level, or filed under a folder that no longer exists.
        for s in self.prefs.sessions.iter().filter(|s| s.folder.as_deref().is_none_or(|f| !known.contains(f))) {
            rows.push(self.session_row(s, 0, cx));
        }
        rows
    }

    fn folder_btn(
        &self,
        id: String,
        label: &str,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        div()
            .id(SharedString::from(id))
            .px(px(4.))
            .text_color(rgb(t.text_secondary))
            .hover(|s| s.text_color(rgb(t.accent)))
            .on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                f(v, cx)
            }))
            .child(label.to_string())
    }

    /// "Sends to: …" above the chat, with the picker.
    pub(super) fn target_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let t = self.theme;
        let (label, unbound) = self.target_line();
        if self.prefs.targets.len() < 2 && !unbound {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_none()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .py(px(4.))
                .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child("Sends to"))
                .child(
                    div().id("target-label").text_size(px(12.)).text_color(rgb(if unbound { t.warning } else { t.text_primary })).child(if unbound {
                        "Its endpoint was deleted. Pick where to send this chat.".to_string()
                    } else {
                        label
                    }),
                )
                .child(self.small("target-menu", "Change", false).on_click(cx.listener(|v, _, _, cx| v.toggle_target_menu(cx)))),
        )
    }
}
