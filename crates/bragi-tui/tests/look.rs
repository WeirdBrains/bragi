//! The look without colour or Unicode, and at sizes where the layout gives way: NO_COLOR draws
//! no colour at all, ASCII borders and icons stand in, the sidebar folds away in a narrow
//! terminal and a tiny one gets a note instead of a broken frame. (Its own test binary: the
//! theme is set once per process.)

use bragi_tui::app::App;
use bragi_tui::session::Session;
use bragi_tui::theme::{self, Depth, Theme};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn screen(app: &mut App, w: u16, h: u16) -> (String, ratatui::buffer::Buffer) {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    app.tick();
    term.draw(|f| bragi_tui::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    let mut s = String::new();
    for y in 0..h {
        for x in 0..w {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    (s, buf)
}

#[test]
fn no_colour_ascii_narrow_and_tiny() {
    let file = weirdnet_themes::builtin("bragi").unwrap();
    theme::init(Theme::new(&file, weirdnet_themes::Variant::Dark, Depth::None, false));
    let session = Session::demo(None).expect("the demo");
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(session.core.clone(), &session.home, dir.path().to_path_buf());

    let (wide, buf) = screen(&mut app, 120, 30);
    assert!(wide.contains("+ Chats") || wide.contains("+-"), "ASCII borders:\n{wide}");
    assert!(!wide.contains('╭') && !wide.contains('│'), "no box drawing without Unicode:\n{wide}");
    assert!(buf.content().iter().all(|c| c.fg == Color::Reset && c.bg == Color::Reset), "NO_COLOR draws no colour");

    // An unlinked demo opens on the link window; the chat is under it.
    app.view = bragi_tui::app::View::Chat;
    let (narrow, _) = screen(&mut app, 70, 24);
    assert!(!narrow.contains("Chats"), "the sidebar folds away when narrow:\n{narrow}");
    assert!(narrow.contains("New chat"), "{narrow}");

    let (tiny, _) = screen(&mut app, 30, 8);
    assert!(tiny.contains("bigger") && tiny.contains("30×8"), "{tiny}");
}
