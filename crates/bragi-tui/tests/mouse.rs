//! The terminal app driven by synthetic mouse events against the demo nodes (memory transport,
//! a test identity, no real network), drawn on ratatui's TestBackend: tabs, hints, buttons,
//! list rows, double clicks, the wheel, hover, links and copy buttons, and the switch that
//! leaves the mouse to the terminal.

use std::time::{Duration, Instant};

use bragi_core::PairState;
use bragi_tui::app::{App, Focus, View};
use bragi_tui::hit::Action;
use bragi_tui::session::Session;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Modifier;

struct Harness {
    app: App,
    term: Terminal<TestBackend>,
    _session: Session,
}

impl Harness {
    fn new(root: &std::path::Path) -> Self {
        let session = Session::demo(None).expect("the demo");
        let app = App::new(session.core.clone(), &session.home, root.to_path_buf());
        let term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        Self { app, term, _session: session }
    }

    fn draw(&mut self) -> String {
        self.app.tick();
        let app = &mut self.app;
        self.term.draw(|f| bragi_tui::ui::draw(f, app)).unwrap();
        let buf = self.term.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn key(&mut self, code: KeyCode) {
        self.app.on_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        self.draw();
    }

    fn typed(&mut self, text: &str) {
        for c in text.chars() {
            self.app.on_event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        self.draw();
    }

    fn until(&mut self, what: &str, until: impl Fn(&App) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let screen = self.draw();
            if screen.contains(what) && until(&self.app) {
                return screen;
            }
            assert!(Instant::now() < deadline, "waited for {what:?}; the screen:\n{screen}\nnotice: {:?}", self.app.notice);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn mouse(&mut self, kind: MouseEventKind, x: u16, y: u16) -> bool {
        self.app.on_event(Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }))
    }

    /// The cell in the middle of the first hit with this action.
    fn spot(&self, action: &Action) -> (u16, u16) {
        let hit = self
            .app
            .hits
            .hits()
            .iter()
            .rev()
            .find(|h| &h.action == action)
            .unwrap_or_else(|| panic!("nothing clickable does {action:?}; there is {:#?}", self.app.hits.hits().iter().map(|h| &h.action).collect::<Vec<_>>()));
        (hit.rect.x + hit.rect.width / 2, hit.rect.y + hit.rect.height / 2)
    }

    /// Clicks the thing that does `action`, then redraws.
    fn click(&mut self, action: Action) {
        let (x, y) = self.spot(&action);
        self.mouse(MouseEventKind::Down(MouseButton::Left), x, y);
        self.mouse(MouseEventKind::Up(MouseButton::Left), x, y);
        self.draw();
    }

    fn double_click(&mut self, action: Action) {
        let (x, y) = self.spot(&action);
        for _ in 0..2 {
            self.mouse(MouseEventKind::Down(MouseButton::Left), x, y);
            self.mouse(MouseEventKind::Up(MouseButton::Left), x, y);
        }
        self.draw();
    }

    /// Links the demo device and waits for the catalog.
    fn linked(root: &std::path::Path) -> Self {
        let mut h = Self::new(root);
        h.until("Link this terminal", |a| a.view == View::Link);
        // The buttons do what the keys did: show a code, then confirm.
        h.click(Action::key('s'));
        h.until("weirdnet-bragi://", |a| matches!(a.state.pair, PairState::Offering { .. }));
        h.click(Action::key('c'));
        assert!(h.app.clipboard.first().is_some_and(|c| c.starts_with("weirdnet-bragi://")));
        h.app.clipboard.clear();
        h.until("Does the other device show this code?", |a| matches!(a.state.pair, PairState::Confirm { .. }));
        h.click(Action::key('y'));
        h.until("online", |a| a.view == View::Chat && a.state.online && a.state.catalog.models.len() >= 2);
        h
    }
}

fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn tabs_hints_windows_and_the_close_button() {
    let dir = root();
    let mut h = Harness::linked(dir.path());
    let screen = h.draw();
    assert!(screen.contains("+ New") && screen.contains("F1"), "{screen}");

    // A tab opens its screen, the corner button closes it, the tab leads back.
    h.click(Action::Goto(View::Settings));
    assert_eq!(h.app.view, View::Settings);
    let screen = h.draw();
    assert!(screen.contains("a Add") && screen.contains("Esc Close"), "{screen}");
    h.click(Action::code(KeyCode::Esc));
    assert_eq!(h.app.view, View::Chat);
    h.click(Action::Goto(View::Status));
    h.until("Inference nodes (2)", |a| a.view == View::Status);
    h.click(Action::key('r'));
    assert!(h.app.notice.as_deref().is_some_and(|n| n.contains("Syncing")));
    // A click outside a window closes it; one inside does nothing.
    h.mouse(MouseEventKind::Down(MouseButton::Left), 60, 20);
    assert_eq!(h.app.view, View::Status);
    h.mouse(MouseEventKind::Down(MouseButton::Left), 1, 5);
    assert_eq!(h.app.view, View::Chat);

    // A hint in the status bar is its key.
    h.click(Action::code(KeyCode::F(1)));
    assert_eq!(h.app.view, View::Help);
    h.click(Action::code(KeyCode::Esc));
    // The footer's model and target open the picker.
    h.click(Action::ctrl('o'));
    assert_eq!(h.app.view, View::Models);
    h.click(Action::code(KeyCode::Esc));
    // + New starts a chat.
    h.click(Action::ctrl('n'));
    assert!(h.app.current.is_none());
}

#[test]
fn chats_the_picker_and_settings_rows() {
    let dir = root();
    let mut h = Harness::linked(dir.path());
    h.typed("Hello from a mouse");
    h.key(KeyCode::Enter);
    h.until("Bragi asks the inference nodes", |a| !a.streaming());
    h.click(Action::ctrl('n'));
    assert!(h.app.current.is_none());

    // One click selects a chat in the sidebar; two open it.
    h.click(Action::Sidebar(0));
    assert_eq!(h.app.focus, Focus::Chats);
    assert!(h.app.current.is_none(), "one click only selects");
    h.double_click(Action::Sidebar(0));
    assert!(h.app.current.is_some());
    assert_eq!(h.app.focus, Focus::Input);

    // The wheel scrolls the transcript, and clicking the message box focuses it.
    let (x, y) = h.spot(&Action::FocusInput);
    assert!(h.mouse(MouseEventKind::ScrollUp, x, 12));
    assert_eq!(h.app.scroll, 3);
    h.mouse(MouseEventKind::ScrollDown, x, 12);
    assert_eq!(h.app.scroll, 0);
    h.app.focus = Focus::Chats;
    h.mouse(MouseEventKind::Down(MouseButton::Left), x, y.max(36));
    h.draw();

    // The picker: a click on the endpoint row selects it; the wheel moves the cursor.
    h.click(Action::Goto(View::Settings));
    h.click(Action::key('a'));
    h.typed("Local");
    h.click(Action::FormField(1));
    h.typed("http://127.0.0.1:9/v1");
    h.click(Action::ctrl('s'));
    h.until("Saved Local", |a| a.settings.endpoints.len() == 1);
    // Settings rows select; a double click sends chats to the endpoint.
    h.click(Action::SettingsRow(0));
    assert_eq!(h.app.settings.sel, 0);
    h.click(Action::code(KeyCode::Esc));
    h.click(Action::ctrl('o'));
    h.until("Where chats go", |a| a.view == View::Models);
    h.click(Action::PickerTarget(1));
    assert_eq!(h.app.picker_target, 1);
    h.click(Action::PickerTarget(0));
    h.click(Action::PickerModel(1));
    assert!(h.app.picker_models_pane && h.app.model_sel == 1);
    let second = h.app.state.catalog.models[1].id.clone();
    h.double_click(Action::PickerModel(1));
    assert_eq!(h.app.view, View::Chat);
    assert_eq!(h.app.model.as_deref(), Some(second.as_str()));
}

#[test]
fn a_question_has_buttons_and_other_clicks_keep_things() {
    let dir = root();
    let mut h = Harness::linked(dir.path());
    h.typed("Delete me");
    h.key(KeyCode::Enter);
    h.until("Bragi asks the inference nodes", |a| !a.streaming());
    h.key(KeyCode::Tab);
    h.app.chat_sel = h.app.rows.iter().position(|r| r.chat_id().is_some()).unwrap();
    h.key(KeyCode::Char('d'));
    let screen = h.draw();
    assert!(screen.contains("y Yes") && screen.contains("n No"), "{screen}");
    h.click(Action::key('n'));
    assert_eq!(h.app.sessions.len(), 1);
    h.key(KeyCode::Char('d'));
    h.draw();
    h.click(Action::key('y'));
    h.until("Deleted", |a| a.sessions.is_empty());
}

#[test]
fn links_and_copy_buttons_in_replies() {
    let dir = root();
    let mut h = Harness::linked(dir.path());
    h.typed("Show me some Markdown");
    h.key(KeyCode::Enter);
    let screen = h.until("What Bragi renders", |a| !a.streaming());
    assert!(screen.contains(" copy "), "the code block has a copy button:\n{screen}");
    let copy = h.app.hits.hits().iter().find_map(|x| matches!(&x.action, Action::Copy(t) if t.contains("fn ") || t.contains("let ") || t.contains("println")).then(|| x.action.clone()));
    let copy = copy.expect("a copy button for the code block");
    h.click(copy.clone());
    let Action::Copy(code) = copy else { unreachable!() };
    assert_eq!(h.app.clipboard.last(), Some(&code));
    // A reply's own copy button copies the reply.
    let reply = h.app.hits.hits().iter().find_map(|x| matches!(&x.action, Action::Copy(t) if t.contains("What Bragi renders")).then(|| x.action.clone()));
    assert!(reply.is_some(), "the reply has a copy button");

    // Links open through the system opener (the loop takes `open_urls`); only web pages and mail.
    h.app.activate(Action::Url("https://example.com/a".into()), false);
    h.app.activate(Action::Url("javascript:alert(1)".into()), false);
    h.app.activate(Action::Url("file:///etc/passwd".into()), false);
    assert_eq!(h.app.open_urls, vec!["https://example.com/a".to_string()]);
}

#[test]
fn hover_tints_and_the_mouse_can_be_left_to_the_terminal() {
    let dir = root();
    let mut h = Harness::linked(dir.path());
    let (x, y) = h.spot(&Action::Goto(View::Code));
    assert!(h.mouse(MouseEventKind::Moved, x, y), "moving onto a button redraws");
    assert!(!h.mouse(MouseEventKind::Moved, x, y), "moving within it doesn't");
    h.draw();
    let buf = h.term.backend().buffer().clone();
    assert!(buf[(x, y)].modifier.contains(Modifier::UNDERLINED), "the hovered button is underlined");
    h.mouse(MouseEventKind::Moved, 60, 20);
    h.draw();
    assert!(!h.term.backend().buffer()[(x, y)].modifier.contains(Modifier::UNDERLINED));

    // F7 leaves the mouse to the terminal (the loop turns reporting off); clicks do nothing then.
    h.key(KeyCode::F(7));
    assert!(!h.app.mouse);
    let (cx, cy) = h.spot(&Action::Goto(View::Settings));
    assert!(!h.mouse(MouseEventKind::Down(MouseButton::Left), cx, cy));
    assert_eq!(h.app.view, View::Chat);
    h.key(KeyCode::F(7));
    assert!(h.app.mouse);
    h.click(Action::Goto(View::Settings));
    assert_eq!(h.app.view, View::Settings);
}

#[test]
fn code_review_buttons() {
    let dir = root();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();
    let mut h = Harness::linked(dir.path());
    h.click(Action::Goto(View::Code));
    h.until("Code suggestions and rewrites", |a| a.view == View::Code);
    h.typed("/add .");
    h.key(KeyCode::Enter);
    h.typed("/rewrite src/lib.rs:1-3 make add saturating");
    h.key(KeyCode::Enter);
    let screen = h.until("Review the proposed change", |a| a.code.phase == bragi_tui::code::Phase::Review);
    assert!(screen.contains("y Accept") && screen.contains("w Write"), "{screen}");
    h.click(Action::key('n'));
    assert_eq!(h.app.code.counts().0, 0, "the Reject button rejected it");
    h.click(Action::key('a'));
    assert_eq!(h.app.code.counts().0, h.app.code.counts().1, "All accepted everything");
    h.click(Action::code(KeyCode::Esc));
    assert_eq!(h.app.code.phase, bragi_tui::code::Phase::Compose);
}
