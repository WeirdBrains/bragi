//! The file picker driven by keys against the demo node and drawn on ratatui's TestBackend:
//! opening it, the tree, filtering, selecting with Space, opening with Enter, the context
//! total against its limit, and staying inside the root.

use bragi_tui::app::{App, View};
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
        let mut app = App::new(session.core.clone(), &session.home, root.to_path_buf());
        app.view = View::Code;
        Self { app, term: Terminal::new(TestBackend::new(110, 32)).unwrap(), _session: session }
    }

    fn draw(&mut self) -> String {
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
        if let Some(dir) = std::env::var_os("BRAGI_PICKER_SHOTS") {
            let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
            let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{n:02}.txt")), &s);
        }
        s
    }

    fn key(&mut self, code: KeyCode) -> String {
        self.app.on_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        self.draw()
    }

    fn typed(&mut self, text: &str) -> String {
        for c in text.chars() {
            self.app.on_event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        self.draw()
    }
}

fn project() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, body) in [
        ("src/main.rs", "fn main() {}\n"),
        ("src/lib.rs", "pub fn add(a: i32, b: i32) -> i32 { a + b }\n"),
        ("README.md", "# demo\n"),
        ("target/debug/junk", "x"),
        ("notes.log", "ignored"),
    ] {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, body).unwrap();
    }
    std::fs::write(d.path().join(".gitignore"), "*.log\n").unwrap();
    d
}

fn context(h: &Harness) -> Vec<String> {
    h.app.code.gathered.files.iter().map(|f| f.path.clone()).collect()
}

#[test]
fn pick_files_into_the_context() {
    let d = project();
    let mut h = Harness::new(d.path());
    // `f` opens it, with the folder's tree, sizes, nothing in the context and no ignored files.
    let s = h.typed("f");
    assert!(h.app.picker.is_some());
    for want in ["Files", "src", "main.rs", "README.md", "0 files, 0 KiB of 512 KiB", "Space select"] {
        assert!(s.contains(want), "{want:?} missing:\n{s}");
    }
    assert!(!s.contains("target") && !s.contains("notes.log") && !s.contains("junk"), "{s}");
    assert!(context(&h).is_empty());

    // Down to src/lib.rs (src is row 0, open; lib.rs row 1), Space adds it and moves on.
    h.key(KeyCode::Down);
    let s = h.typed(" ");
    assert_eq!(context(&h), ["src/lib.rs"]);
    assert!(s.contains("1 file, 1 KiB of 512 KiB"), "{s}");
    // Space again on main.rs, then back up and off lib.rs.
    h.typed(" ");
    assert_eq!(context(&h), ["src/lib.rs", "src/main.rs"]);
    h.key(KeyCode::Up);
    h.key(KeyCode::Up);
    h.typed(" ");
    assert_eq!(context(&h), ["src/main.rs"]);

    // A filter narrows to the matches; Enter on a file adds it and closes the picker.
    h.typed("/");
    let s = h.typed("read");
    assert!(s.contains("README.md") && !s.contains("main.rs"), "{s}");
    h.key(KeyCode::Enter);
    h.key(KeyCode::Enter);
    assert!(h.app.picker.is_none());
    assert_eq!(context(&h), ["src/main.rs", "README.md"]);
    // The context sits in the sidebar or the status; nothing outside the root is ever listed.
    assert!(context(&h).iter().all(|p| !p.starts_with("..") && !p.starts_with('/')));
}

#[test]
fn escape_closes_and_a_folder_takes_all_its_files() {
    let d = project();
    let mut h = Harness::new(d.path());
    h.app.on_event(Event::Key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL)));
    assert!(h.app.picker.is_some());
    // Space on the `src` folder adds both of its files.
    h.typed(" ");
    assert_eq!(context(&h), ["src/lib.rs", "src/main.rs"]);
    let s = h.draw();
    assert!(s.contains("2 files"), "{s}");
    // And again takes them out.
    h.key(KeyCode::Up);
    h.key(KeyCode::Up);
    h.key(KeyCode::Up);
    h.typed(" ");
    assert!(context(&h).is_empty());
    h.key(KeyCode::Esc);
    assert!(h.app.picker.is_none());
    // Typed as a command it opens too.
    h.typed("/files");
    h.key(KeyCode::Enter);
    assert!(h.app.picker.is_some());
}

#[test]
fn the_limit_is_shown_and_kept() {
    let d = project();
    for i in 0..3 {
        std::fs::write(d.path().join(format!("big{i}.txt")), "z".repeat(200 * 1024)).unwrap();
    }
    let mut h = Harness::new(d.path());
    h.typed("f/big");
    h.typed(" ");
    h.key(KeyCode::Down);
    h.typed(" ");
    assert_eq!(context(&h).len(), 2);
    h.key(KeyCode::Down);
    let s = h.typed(" ");
    assert_eq!(context(&h).len(), 2, "a third 200 KiB file doesn't fit");
    assert!(s.contains("400 KiB of 512 KiB"), "{s}");
    assert!(h.app.notice.as_deref().is_some_and(|n| n.contains("past 512 KiB")), "{:?}", h.app.notice);
}
