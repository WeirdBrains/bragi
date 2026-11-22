//! Sessions through `Core`: each is bound to a target and keeps its own history; deleting an
//! endpoint rebinds or unbinds its sessions and never deletes history; nested folders; Weirdnet
//! links (add, edit, delete).

mod common;

use bragi_core::{ChatEvent, Core, EndpointInput, EndpointMode, ErrorKind, Target};
use common::*;

fn input(name: &str, url: &str) -> EndpointInput {
    EndpointInput { name: name.into(), base_url: url.into(), model: Some("mock-1".into()), mode: EndpointMode::Alongside, enabled: true }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_session_keeps_its_target_and_history_across_deletes() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    let m1 = mock_openai(None, vec!["mock-1"], Chat::Words(vec!["one"])).await;
    let m2 = mock_openai(None, vec!["mock-1"], Chat::Words(vec!["two"])).await;
    let e1 = core.add_endpoint(input("First", &m1.url), None).unwrap();
    let e2 = core.add_endpoint(input("Second", &m2.url), None).unwrap();
    let t = |id: &str| Target::Endpoint { id: id.into(), model: "mock-1".into() };

    let mut pool = core.send(None, "to the pool", None).unwrap();
    drain(&mut pool).await;
    let mut a = core.send_to(None, "to first", None, Vec::new(), Some(t(&e1.id))).unwrap();
    assert_eq!(answer_text(&drain(&mut a).await), "one");
    let mut b = core.send_to(None, "to second", None, Vec::new(), Some(t(&e2.id))).unwrap();
    assert_eq!(answer_text(&drain(&mut b).await), "two");
    let mut a2 = core.send_to(Some(&a.conversation_id), "more", None, Vec::new(), None).unwrap();
    assert_eq!(answer_text(&drain(&mut a2).await), "one", "a bound session goes where it is bound");

    let list = core.sessions(0, 20).unwrap();
    let find = |id: &str| list.iter().find(|s| s.id == id).unwrap().clone();
    assert_eq!(find(&pool.conversation_id).target, Some(Target::Pool));
    assert_eq!(find(&a.conversation_id).target_label, "First · mock-1");
    assert_eq!(find(&b.conversation_id).target_label, "Second · mock-1");
    assert_eq!(core.endpoint_sessions(&e1.id), 1);
    // Histories are separate.
    assert_eq!(core.conversation(&a.conversation_id).unwrap().messages.len(), 4);
    assert_eq!(core.conversation(&b.conversation_id).unwrap().messages.len(), 2);

    // Delete First with Second as the new home: its session moves; history stays whole.
    let r = core.delete_endpoint(&e1.id, Some(t(&e2.id))).unwrap();
    assert_eq!((r.rebound, r.unbound), (1, 0));
    assert_eq!(core.conversation(&a.conversation_id).unwrap().messages.len(), 4);
    let mut again = core.send_to(Some(&a.conversation_id), "after move", None, Vec::new(), None).unwrap();
    assert_eq!(answer_text(&drain(&mut again).await), "two");

    // Delete Second with nowhere to go: its sessions are unbound, history stays, sending asks.
    let r = core.delete_endpoint(&e2.id, None).unwrap();
    assert_eq!((r.rebound, r.unbound), (0, 2));
    let s = core.sessions(0, 20).unwrap();
    let ub = s.iter().find(|x| x.id == b.conversation_id).unwrap();
    assert!(ub.unbound && ub.target.is_none() && ub.target_label.contains("deleted"), "{ub:?}");
    assert_eq!(core.conversation(&b.conversation_id).unwrap().messages.len(), 2);
    assert_eq!(core.send_to(Some(&b.conversation_id), "x", None, Vec::new(), None).err().unwrap().kind, ErrorKind::BadRequest);
    // Binding it to the pool fixes it; it then sends over the pool.
    core.bind_session(&b.conversation_id, Target::Pool).unwrap();
    let mut h = core.send_to(Some(&b.conversation_id), "back on the pool", None, Vec::new(), None).unwrap();
    let ev = drain(&mut h).await;
    assert!(matches!(ev.last(), Some(ChatEvent::Done { .. })));
    assert!(!answer_text(&ev).is_empty() && answer_text(&ev) != "two");
    assert_eq!(core.bind_session(&b.conversation_id, t("ep-gone")).unwrap_err().kind, ErrorKind::NotFound);
    // Rebinding to a deleted endpoint, or to the endpoint itself, is refused up front.
    let e3 = core.add_endpoint(input("Third", &m1.url), None).unwrap();
    assert!(core.delete_endpoint(&e3.id, Some(t(&e3.id))).is_err());
    assert!(core.delete_endpoint(&e3.id, Some(t("ep-gone"))).is_err());
    assert_eq!(core.endpoints().len(), 1, "a refused delete deletes nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folders_nest_and_hold_sessions_and_survive_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, demo) = linked(tmp.path()).await;
    let mut h = core.send(None, "filed", None).unwrap();
    drain(&mut h).await;
    let conv = h.conversation_id.clone();

    let work = core.create_folder("Work", None).unwrap();
    let reports = core.create_folder("Reports", Some(&work.id)).unwrap();
    let mine = core.create_folder("Mine", None).unwrap();
    core.set_session_folder(&conv, Some(&reports.id)).unwrap();
    assert_eq!(core.sessions(0, 10).unwrap()[0].folder.as_deref(), Some(reports.id.as_str()));
    assert!(core.set_session_folder("c-nope", None).is_err());
    assert!(core.set_session_folder(&conv, Some("f-nope")).is_err());

    core.rename_folder(&reports.id, "Quarterly").unwrap();
    core.move_folder(&reports.id, Some(&mine.id)).unwrap();
    assert!(core.move_folder(&mine.id, Some(&reports.id)).is_err(), "a folder can't go inside its own subfolder");
    let names: Vec<(String, Option<String>)> = core.folders().into_iter().map(|f| (f.name, f.parent)).collect();
    assert!(names.contains(&("Quarterly".into(), Some(mine.id.clone()))));

    // Restart: folders and filing are still there.
    drop(core);
    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    assert_eq!(core.folders().len(), 3);
    until(&core, "online", |s| s.online).await;
    assert_eq!(core.sessions(0, 10).unwrap()[0].folder.as_deref(), Some(reports.id.as_str()));

    // Deleting folders moves their contents up and never deletes the conversation.
    core.delete_folder(&reports.id).unwrap();
    assert_eq!(core.sessions(0, 10).unwrap()[0].folder.as_deref(), Some(mine.id.as_str()), "up to the parent");
    core.delete_folder(&mine.id).unwrap();
    assert_eq!(core.sessions(0, 10).unwrap()[0].folder, None);
    core.delete_folder(&work.id).unwrap();
    assert_eq!(core.conversation(&conv).unwrap().messages.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn weirdnet_links_add_edit_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    let a = "/ip4/192.0.2.7/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY";
    let b = "/ip4/192.0.2.8/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY";
    core.add_node(a).unwrap();
    assert_eq!(core.links().len(), 1);
    assert!(!core.links()[0].online && core.links()[0].address == a);
    core.edit_link(a, b).unwrap();
    assert_eq!(core.links().iter().map(|l| l.address.as_str()).collect::<Vec<_>>(), [b]);
    assert!(core.edit_link(b, "garbage").is_err(), "a bad edit changes nothing");
    assert_eq!(core.links().len(), 1);
    assert_eq!(core.remove_link(a).unwrap_err().kind, ErrorKind::NotFound);
    core.remove_link(b).unwrap();
    assert!(core.links().is_empty());
}
