//! The Theme screen driven key by key: the picker previews live and Esc puts the look back,
//! Enter saves the choice with Bragi's settings, the editor changes a copy of a built-in with a
//! live preview and saves it, and duplicate, rename, delete and export work. (Its own test
//! binary: the theme is process-wide.)

use bragi_tui::app::{App, View};
use bragi_tui::session::Session;
use bragi_tui::theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use weirdnet_themes::Variant;

struct Harness {
    app: App,
    term: Terminal<TestBackend>,
    session: Session,
}

impl Harness {
    fn new() -> Self {
        let session = Session::demo(None).expect("the demo");
        let app = App::new(session.core.clone(), &session.home, std::env::temp_dir());
        Self { app, term: Terminal::new(TestBackend::new(120, 40)).unwrap(), session }
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

    fn key(&mut self, code: KeyCode) -> String {
        self.app.on_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        self.draw()
    }

    fn typed(&mut self, text: &str) -> String {
        for c in text.chars() {
            self.key(KeyCode::Char(c));
        }
        self.draw()
    }

    fn pref(&self) -> bragi_core::Preference {
        self.session.core.appearance().preference()
    }
}

#[test]
fn pick_preview_save_edit_duplicate_rename_delete_export() {
    // SAFETY: the only test in this binary, before any thread of its own.
    unsafe { std::env::set_var("BRAGI_TUI_COLOR", "truecolor") };
    let mut h = Harness::new();
    let a = h.session.core.appearance();
    a.set_theme("bragi").unwrap();
    a.set_mode(bragi_core::ModePref::Dark).unwrap();
    theme::init(theme::build(&a.load("bragi").unwrap(), bragi_core::ModePref::Dark));

    // The menu: F7 opens the screen; the header has the tab.
    let s = h.key(KeyCode::F(7));
    assert_eq!(h.app.view, View::Themes);
    assert!(s.contains("Theme") && s.contains("Bragi") && s.contains("Dracula"), "{s}");

    // Moving previews live, Esc puts the look back and saves nothing.
    let start = theme::get().name.clone();
    h.key(KeyCode::Up);
    assert_ne!(theme::get().name, start);
    h.key(KeyCode::Esc);
    assert_eq!(theme::get().name, start);
    assert_eq!(h.pref().theme, "bragi");

    // Enter saves the choice; m cycles the mode first.
    h.key(KeyCode::F(7));
    let i = h.app.themes.rows.iter().position(|r| r.name == "nord").unwrap();
    while h.app.themes.sel != i {
        h.key(if h.app.themes.sel < i { KeyCode::Down } else { KeyCode::Up });
    }
    // dark -> light -> system -> dark
    h.key(KeyCode::Char('m'));
    assert_eq!(h.app.themes.mode, bragi_core::ModePref::Light);
    h.key(KeyCode::Char('m'));
    h.key(KeyCode::Char('m'));
    assert_eq!(h.app.themes.mode, bragi_core::ModePref::Dark);
    h.key(KeyCode::Enter);
    assert_ne!(h.app.view, View::Themes);
    assert_eq!(h.pref().theme, "nord");
    assert_eq!(theme::get().name, "nord");

    // Edit a built-in: a copy is made and edited, with a live preview, then saved.
    h.key(KeyCode::F(7));
    h.key(KeyCode::Char('e'));
    assert!(h.app.themes.edit.is_some());
    let name = h.app.themes.edit.as_ref().unwrap().name.clone();
    assert_eq!(name, "nord-custom");
    let s = h.draw();
    assert!(s.contains("nord-custom") && s.contains("accent"), "{s}");
    // Down to `accent` (the 8th role), type a colour.
    let accent = bragi_core::appearance::roles().iter().position(|r| *r == "accent").unwrap();
    for _ in 0..accent {
        h.key(KeyCode::Down);
    }
    h.key(KeyCode::Enter);
    for _ in 0..8 {
        h.key(KeyCode::Backspace);
    }
    h.typed("#ff0000");
    assert_eq!(theme::get().accent, ratatui::style::Color::Rgb(255, 0, 0), "live while typing");
    h.key(KeyCode::Enter);
    h.key(KeyCode::Char('s'));
    let want = a.palette_for(&name, h.app.themes.edit.as_ref().unwrap().variant).unwrap();
    assert_eq!(want.get("accent"), Some("#ff0000"));
    assert!(h.app.themes.msg.as_deref().is_some_and(|m| m.starts_with("Saved")), "{:?}", h.app.themes.msg);
    // A bad colour is refused and Esc puts the old one back.
    h.key(KeyCode::Enter);
    h.typed("zz");
    h.key(KeyCode::Enter);
    assert!(h.app.themes.msg.as_deref().is_some_and(|m| m.contains("#rgb")));
    h.key(KeyCode::Esc);
    assert_eq!(theme::get().accent, ratatui::style::Color::Rgb(255, 0, 0));
    // Leaving a clean editor goes back to the list; the shipped nord is untouched.
    h.key(KeyCode::Esc);
    assert!(h.app.themes.edit.is_none());
    assert_ne!(a.palette_for("nord", Variant::Dark).unwrap().get("accent"), Some("#ff0000"));

    // Duplicate (d, a name, Enter), rename (r), export (E), delete (x then y).
    let sel = h.app.themes.rows.iter().position(|r| r.name == name).unwrap();
    while h.app.themes.sel != sel {
        h.key(KeyCode::Down);
    }
    h.key(KeyCode::Char('d'));
    for _ in 0..30 {
        h.key(KeyCode::Backspace);
    }
    h.typed("mine");
    h.key(KeyCode::Enter);
    assert!(a.rows().unwrap().iter().any(|r| r.name == "mine"));
    h.key(KeyCode::Char('r'));
    for _ in 0..30 {
        h.key(KeyCode::Backspace);
    }
    h.typed("ours");
    h.key(KeyCode::Enter);
    assert!(a.rows().unwrap().iter().any(|r| r.name == "ours") && !a.rows().unwrap().iter().any(|r| r.name == "mine"));
    let out = h.session.home.join("ours.toml");
    h.key(KeyCode::Char('E'));
    for _ in 0..40 {
        h.key(KeyCode::Backspace);
    }
    h.typed(out.to_str().unwrap());
    h.key(KeyCode::Enter);
    assert!(std::fs::read_to_string(&out).unwrap().contains("ours"));
    h.key(KeyCode::Char('x'));
    h.key(KeyCode::Char('y'));
    assert!(!a.rows().unwrap().iter().any(|r| r.name == "ours"));
    // A built-in can't be deleted or renamed.
    let i = h.app.themes.rows.iter().position(|r| r.name == "gruvbox").unwrap();
    while h.app.themes.sel != i {
        if h.app.themes.sel < i {
            h.key(KeyCode::Down);
        } else {
            h.key(KeyCode::Up);
        }
    }
    h.key(KeyCode::Char('x'));
    assert!(h.app.themes.msg.as_deref().is_some_and(|m| m.contains("built-in")));
    // Import what was exported.
    h.key(KeyCode::Char('i'));
    h.typed(out.to_str().unwrap());
    h.key(KeyCode::Enter);
    assert!(a.rows().unwrap().iter().any(|r| r.name == "ours"));
    h.key(KeyCode::Esc);
    assert_ne!(h.app.view, View::Themes);
    assert_eq!(theme::get().name, "nord", "the saved choice is back");
}
