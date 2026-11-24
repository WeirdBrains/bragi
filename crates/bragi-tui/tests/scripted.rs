//! The terminal app driven key by key against the demo nodes (memory transport, a test
//! identity, no real network), drawn on ratatui's TestBackend: link, chat with Markdown and
//! reasoning, attachments, folders, endpoints in Settings, targets and models, status, help,
//! and code mode from request to written file and undo.

use std::time::{Duration, Instant};

use bragi_core::PairState;
use bragi_tui::app::{App, View};
use bragi_tui::code::Phase;
use bragi_tui::session::Session;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

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

    fn ctrl(&mut self, c: char) {
        self.app.on_event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)));
        self.draw();
    }

    fn typed(&mut self, text: &str) {
        for c in text.chars() {
            self.app.on_event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        self.draw();
    }

    /// Draws until the screen shows `what` (or `until` holds), failing with the screen after 30 s.
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
}

#[test]
fn link_chat_models_status_help_and_code() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/lib.rs"), "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();
    std::fs::write(project.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir_all(project.path().join("target")).unwrap();
    std::fs::write(project.path().join("target/junk.rs"), "junk").unwrap();
    let mut h = Harness::new(project.path());

    // Not linked: the link screen. s shows a code, c copies it (OSC 52), the demo device
    // opens it, and both sides confirm the same check code.
    let screen = h.until("Link this terminal", |a| a.view == View::Link);
    assert!(screen.contains("Press s to show a link code"));
    h.key(KeyCode::Char('s'));
    let screen = h.until("weirdnet-bragi://", |a| matches!(a.state.pair, PairState::Offering { .. }));
    assert!(screen.contains("c copies it"));
    h.key(KeyCode::Char('c'));
    assert!(h.app.clipboard.first().is_some_and(|c| c.starts_with("weirdnet-bragi://")), "the code went to the clipboard");
    h.app.clipboard.clear();
    h.until("Does the other device show this code?", |a| matches!(a.state.pair, PairState::Confirm { .. }));
    h.key(KeyCode::Char('y'));
    let screen = h.until("online", |a| a.view == View::Chat && a.state.online && a.state.catalog.models.len() >= 2);
    assert!(screen.contains("New chat with Weirdnet pool"), "{screen}");

    // Chat: the answer streams in and lands in the chat list.
    h.typed("Hello from a terminal");
    h.key(KeyCode::Enter);
    assert!(h.app.streaming());
    let screen = h.until("Bragi asks the inference nodes", |a| !a.streaming());
    assert!(screen.contains("You"));
    assert!(screen.contains("Hello from a terminal"), "the chat list and transcript show it:\n{screen}");

    // Markdown: drawn, not shown as marks.
    h.typed("Show me some Markdown");
    h.key(KeyCode::Enter);
    let screen = h.until("What Bragi renders", |a| !a.streaming());
    assert!(!screen.contains("## What Bragi renders") && screen.contains("• ") && screen.contains(" rust "), "{screen}");
    // The reply is labelled, and the status bar shows the connection.
    assert!(screen.contains("◆ Bragi") && screen.contains("● online"), "{screen}");

    // Reasoning: a folded Thinking line; Ctrl-K shows the reasoning.
    h.typed(&format!("{} which node?", bragi_core::demo::THINK_PROMPT));
    h.key(KeyCode::Enter);
    let screen = h.until("Thought for", |a| !a.streaming());
    assert!(screen.contains("▸ Thought for"), "{screen}");
    h.ctrl('k');
    h.until("┊ ", |a| a.show_thinking);
    h.ctrl('k');

    // Attachments: a text file goes with the next message.
    std::fs::write(project.path().join("notes.txt"), "line one\nline two\n").unwrap();
    h.typed(&format!("/attach {}", project.path().join("notes.txt").display()));
    h.key(KeyCode::Enter);
    assert_eq!(h.app.attachments.len(), 1);
    h.typed("What do my notes say?");
    h.key(KeyCode::Enter);
    let screen = h.until("I read notes.txt", |a| !a.streaming());
    assert!(screen.contains("attached notes.txt (2 lines)"), "{screen}");

    // Training consent: off until /train, shown in the chat's title, withdrawn with /train off.
    assert!(!h.app.current.as_ref().unwrap().training);
    h.typed("/train");
    h.key(KeyCode::Enter);
    let screen = h.until("Marked for training", |a| a.current.as_ref().is_some_and(|c| c.training));
    assert!(screen.contains("· training"), "{screen}");
    h.typed("/train off");
    h.key(KeyCode::Enter);
    h.until("No longer used for training", |a| a.current.as_ref().is_some_and(|c| !c.training));

    // Folders: make one, move this chat into it, fold it in the sidebar.
    h.typed("/folder Projects");
    h.key(KeyCode::Enter);
    h.until("▾ Projects", |a| a.folders.len() == 1);
    h.typed("/move projects");
    h.key(KeyCode::Enter);
    let screen = h.until("Moved to projects", |a| a.sessions.iter().any(|s| s.folder.is_some()));
    // The folder row: chevron, name, and its count on the right.
    let row = screen.lines().find(|l| l.contains("▾ Projects")).unwrap_or_default();
    assert!(row.split('│').nth(1).unwrap_or(row).trim_end().ends_with('1'), "{screen}");
    h.ctrl('b');
    assert_eq!(h.app.focus, bragi_tui::app::Focus::Chats);
    h.app.chat_sel = h.app.rows.iter().position(|r| r.folder_id().is_some()).unwrap();
    h.key(KeyCode::Enter);
    h.until("▸ Projects", |a| a.closed.len() == 1);
    h.key(KeyCode::Enter);
    h.ctrl('b');

    // Settings: add an endpoint; the key is typed masked and never drawn.
    h.ctrl('p');
    h.until("No endpoints", |a| a.view == View::Settings);
    h.key(KeyCode::Char('a'));
    h.typed("Local");
    h.key(KeyCode::Tab);
    h.typed("http://127.0.0.1:9/v1");
    h.key(KeyCode::Tab);
    h.typed("small-model");
    h.key(KeyCode::Tab);
    h.typed("sk-very-secret");
    let screen = h.draw();
    assert!(screen.contains("••••••••••••••") && !screen.contains("sk-very-secret"), "{screen}");
    h.key(KeyCode::Enter);
    let screen = h.until("Saved Local", |a| a.settings.endpoints.len() == 1);
    assert!(screen.contains("key set") && screen.contains("next to the pool") && !screen.contains("sk-very-secret"), "{screen}");
    h.key(KeyCode::Esc);

    // Targets and models: the endpoint is offered next to the pool; pick the pool's second model.
    h.ctrl('o');
    let screen = h.until("Where chats go", |a| a.view == View::Models);
    assert!(screen.contains("Local · small-model") && screen.contains(bragi_core::demo::WARM_MODEL), "{screen}");
    let second = h.app.state.catalog.models[1].id.clone();
    h.key(KeyCode::Down);
    h.until("Models on this endpoint", |a| a.picker_target == 1);
    h.key(KeyCode::Up);
    h.key(KeyCode::Tab);
    h.key(KeyCode::Down);
    h.key(KeyCode::Enter);
    assert_eq!(h.app.model.as_deref(), Some(second.as_str()));
    h.until(&format!("│ {second} │"), |a| a.view == View::Chat);
    h.key(KeyCode::Char('?'));
    h.app.model = None;

    // Help lists the keys; status lists nodes and models.
    let mut screen = h.until("Keyboard", |a| a.view == View::Help);
    // The help is longer than the window: read it all.
    for _ in 0..4 {
        h.key(KeyCode::PageDown);
        screen.push_str(&h.draw());
    }
    assert!(screen.contains("Ctrl-L, F2") && screen.contains("/rewrite FILE[:A-B] how") && screen.contains("/folder NAME"), "{screen}");
    h.key(KeyCode::Esc);
    h.ctrl('t');
    let screen = h.until("Inference nodes (2)", |a| a.view == View::Status);
    assert!(screen.contains("Online        yes"), "{screen}");
    h.key(KeyCode::Esc);
    assert_eq!(h.app.view, View::Chat);

    // Code mode: add the folder (target/ is ignored), rewrite two lines, review, write, undo.
    h.ctrl('e');
    h.until("Code · ", |a| a.view == View::Code);
    h.typed("/add .");
    h.key(KeyCode::Enter);
    let screen = h.until("Context (2 files", |_| true);
    assert!(screen.contains("src/lib.rs") && !screen.contains("junk.rs"), "{screen}");
    h.typed("/rewrite src/lib.rs:2-2 explain the sum");
    h.key(KeyCode::Enter);
    let screen = h.until("change 1/1", |a| a.code.phase == Phase::Review);
    assert!(screen.contains("explain the sum") && screen.contains("✓ accepted"), "{screen}");
    h.key(KeyCode::Char('v'));
    h.until("│", |a| a.code.side_by_side);
    h.key(KeyCode::Char('w'));
    let lib = std::fs::read_to_string(project.path().join("src/lib.rs")).unwrap();
    assert!(lib.contains(": explain the sum\n    a + b\n"), "{lib}");
    h.until("Wrote 1 file", |a| a.code.phase == Phase::Compose);
    h.typed("/undo");
    h.key(KeyCode::Enter);
    h.until("Undid the last change", |_| true);
    assert_eq!(std::fs::read_to_string(project.path().join("src/lib.rs")).unwrap(), "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n");

    // A suggestion rejected leaves the file alone.
    h.typed("tidy this up");
    h.key(KeyCode::Enter);
    h.until("change 1/1", |a| a.code.phase == Phase::Review);
    h.key(KeyCode::Char('n'));
    h.key(KeyCode::Char('w'));
    h.until("Nothing accepted", |_| true);
    assert!(!std::fs::read_to_string(project.path().join("src/lib.rs")).unwrap().contains("//"));

    h.ctrl('q');
    assert!(h.app.quit);
}
