use speedsheet_core::shared::{Hub, Presence};
use speedsheet_core::{io, Workbook};

fn book_at(path: &std::path::Path) -> Workbook {
    let mut wb = Workbook::new();
    wb.set_inputs(0, 0, 0, &[vec!["a".into(), "b".into()]]).unwrap();
    io::save(&mut wb, path, 0).unwrap();
    io::open(path).unwrap()
}

#[test]
fn second_window_finds_the_open_file_and_shares_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.xlsx");
    let mut hub = Hub::new();
    hub.attach_new("main", book_at(&path));
    // Another spelling of the same path still matches.
    let other_spelling = dir.path().join(".").join("book.xlsx");
    assert_eq!(hub.views_with_file(&other_spelling, "w2"), vec!["main".to_string()]);
    // The view itself does not count.
    assert!(hub.views_with_file(&path, "main").is_empty());

    hub.attach_shared("w2", "main").unwrap();
    assert_eq!(hub.peers_of("main"), vec!["w2".to_string()]);
    // An edit through one view is seen through the other: it is the same workbook.
    hub.doc("w2").lock().unwrap().set_inputs(0, 1, 0, &[vec!["from w2".into()]]).unwrap();
    {
        let main = hub.doc("main");
        let wb = main.lock().unwrap();
        assert_eq!(wb.sheets[0].value(1, 0).display(), "from w2");
        assert!(wb.modified);
    }
    // Saving from the first window writes the second window's edit too.
    {
        let main = hub.doc("main");
        let mut wb = main.lock().unwrap();
        io::save(&mut wb, &path, 0).unwrap();
    }
    let back = io::open(&path).unwrap();
    assert_eq!(back.sheets[0].value(1, 0).display(), "from w2");
}

#[test]
fn numbers_presence_and_closing() {
    let mut hub = Hub::new();
    hub.doc("a");
    hub.attach_shared("b", "a").unwrap();
    hub.attach_shared("c", "a").unwrap();
    let p = Presence { sheet: 0, row: 4, col: 2, r0: 4, c0: 2, r1: 6, c1: 3 };
    hub.set_presence("b", Some(p));
    let seen = hub.peers("a");
    assert_eq!((seen.number, seen.count), (1, 3));
    assert_eq!(seen.peers.len(), 2);
    assert_eq!(seen.peers[0].view, "b");
    assert_eq!(seen.peers[0].number, 2);
    assert_eq!(seen.peers[0].presence, Some(p));
    assert_eq!(seen.peers[1].presence, None);

    // Closing a window renumbers the rest and tells who is left.
    assert_eq!(hub.detach("a"), vec!["b".to_string(), "c".to_string()]);
    let seen = hub.peers("c");
    assert_eq!((seen.number, seen.count), (2, 2));
    assert_eq!(seen.peers[0].number, 1);

    // Opening something else in a view leaves the shared document to the others.
    assert_eq!(hub.attach_new("b", Workbook::new()), vec!["c".to_string()]);
    assert_eq!(hub.peers("c").count, 1);
    assert!(hub.peers_of("b").is_empty());
}

#[test]
fn save_as_moves_the_file_identity() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.xlsx");
    let second = dir.path().join("second.xlsx");
    let mut hub = Hub::new();
    hub.attach_new("main", book_at(&first));
    {
        let doc = hub.doc("main");
        let mut wb = doc.lock().unwrap();
        io::save(&mut wb, &second, 0).unwrap();
    }
    hub.set_path("main", Some(&second));
    assert!(hub.views_with_file(&first, "x").is_empty());
    assert_eq!(hub.views_with_file(&second, "x"), vec!["main".to_string()]);
}

#[test]
fn closed_documents_are_dropped() {
    let mut hub = Hub::new();
    let doc = hub.doc("a");
    assert_eq!(std::sync::Arc::strong_count(&doc), 2);
    hub.detach("a");
    assert_eq!(std::sync::Arc::strong_count(&doc), 1);
    assert!(!hub.has_view("a"));
    assert!(hub.attach_shared("b", "a").is_none());
}
