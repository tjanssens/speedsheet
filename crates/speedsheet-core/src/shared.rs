//! Shared documents: several views (windows) can show the same workbook.
//!
//! The hub maps each view to a document. When a file is opened that another view already
//! shows, the front-end can attach to that document instead of loading a second copy, so
//! every edit lands in one workbook and saving from any window keeps every window's edits.
//! The hub also keeps where each view's cursor is, so the others can draw it.
//!
//! Locking: the hub itself is meant to sit behind one short-lived lock; each document has its
//! own lock, so a slow operation in one window never blocks other documents.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::Workbook;

pub type SharedDoc = Arc<Mutex<Workbook>>;

/// Where a view's cursor is: the active cell and the selected range, in view coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub sheet: usize,
    pub row: usize,
    pub col: usize,
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

/// Another view on the same document, as one view sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub view: String,
    /// 1-based position among the views on the document (the ":2" in "Book.xlsx:2").
    pub number: usize,
    pub presence: Option<Presence>,
}

/// What a view needs to know about the other views on its document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peers {
    /// This view's own 1-based number.
    pub number: usize,
    /// Number of views on the document, this one included.
    pub count: usize,
    pub peers: Vec<Peer>,
}

struct DocEntry {
    doc: SharedDoc,
    /// Identity of the file on disk, see [`file_key`].
    key: Option<PathBuf>,
    /// Views in the order they attached.
    views: Vec<String>,
}

struct ViewEntry {
    doc: u64,
    presence: Option<Presence>,
}

#[derive(Default)]
pub struct Hub {
    docs: HashMap<u64, DocEntry>,
    views: HashMap<String, ViewEntry>,
    next: u64,
}

/// A comparable identity for a file path: canonical (symlinks and `..` resolved), and
/// case-insensitive on Windows and macOS, whose default file systems are.
pub fn file_key(path: &Path) -> PathBuf {
    let p = std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
        }
    });
    if cfg!(any(windows, target_os = "macos")) {
        PathBuf::from(p.to_string_lossy().to_lowercase())
    } else {
        p
    }
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// The document `view` shows. A view seen for the first time gets a new empty workbook.
    pub fn doc(&mut self, view: &str) -> SharedDoc {
        if !self.views.contains_key(view) {
            self.attach_new(view, Workbook::new());
        }
        let id = self.views[view].doc;
        self.docs[&id].doc.clone()
    }

    /// Whether the hub knows `view`.
    pub fn has_view(&self, view: &str) -> bool {
        self.views.contains_key(view)
    }

    /// Show `wb` in `view` as a document of its own, replacing what the view showed.
    /// Returns the views that shared the view's previous document.
    pub fn attach_new(&mut self, view: &str, wb: Workbook) -> Vec<String> {
        let old_peers = self.detach(view);
        let id = self.next;
        self.next += 1;
        let key = wb.path.as_deref().map(file_key);
        self.docs.insert(id, DocEntry { doc: Arc::new(Mutex::new(wb)), key, views: vec![view.to_string()] });
        self.views.insert(view.to_string(), ViewEntry { doc: id, presence: None });
        old_peers
    }

    /// Show the document of `other` in `view` too. Returns the views that shared the view's
    /// previous document, or None when `other` is unknown.
    pub fn attach_shared(&mut self, view: &str, other: &str) -> Option<Vec<String>> {
        let id = self.views.get(other)?.doc;
        if self.views.get(view).map(|v| v.doc) == Some(id) {
            return Some(Vec::new());
        }
        let old_peers = self.detach(view);
        self.docs.get_mut(&id)?.views.push(view.to_string());
        self.views.insert(view.to_string(), ViewEntry { doc: id, presence: None });
        Some(old_peers)
    }

    /// Forget `view` (its window closed). A document no view shows any more is dropped.
    /// Returns the views that shared its document.
    pub fn detach(&mut self, view: &str) -> Vec<String> {
        let Some(v) = self.views.remove(view) else { return Vec::new() };
        let Some(d) = self.docs.get_mut(&v.doc) else { return Vec::new() };
        d.views.retain(|x| x != view);
        if d.views.is_empty() {
            self.docs.remove(&v.doc);
            Vec::new()
        } else {
            d.views.clone()
        }
    }

    /// The other views on the same document as `view`.
    pub fn peers_of(&self, view: &str) -> Vec<String> {
        self.views
            .get(view)
            .and_then(|v| self.docs.get(&v.doc))
            .map(|d| d.views.iter().filter(|x| *x != view).cloned().collect())
            .unwrap_or_default()
    }

    /// Views other than `except` that show the file at `path`, in the order they opened it.
    pub fn views_with_file(&self, path: &Path, except: &str) -> Vec<String> {
        let key = file_key(path);
        let mut ids: Vec<_> = self.docs.iter().filter(|(_, d)| d.key.as_ref() == Some(&key)).map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.iter().flat_map(|id| self.docs[id].views.iter()).filter(|v| *v != except).cloned().collect()
    }

    /// Record that the document `view` shows now lives at `path` (after Save As).
    pub fn set_path(&mut self, view: &str, path: Option<&Path>) {
        if let Some(d) = self.views.get(view).and_then(|v| self.docs.get_mut(&v.doc)) {
            d.key = path.map(file_key);
        }
    }

    pub fn set_presence(&mut self, view: &str, presence: Option<Presence>) {
        if let Some(v) = self.views.get_mut(view) {
            v.presence = presence;
        }
    }

    /// The other views on `view`'s document, with their numbers and cursors.
    pub fn peers(&self, view: &str) -> Peers {
        let Some(d) = self.views.get(view).and_then(|v| self.docs.get(&v.doc)) else {
            return Peers { number: 1, count: 1, peers: Vec::new() };
        };
        let number = d.views.iter().position(|x| x == view).map_or(1, |i| i + 1);
        let peers = d
            .views
            .iter()
            .enumerate()
            .filter(|(_, x)| *x != view)
            .map(|(i, x)| Peer { view: x.clone(), number: i + 1, presence: self.views.get(x).and_then(|v| v.presence) })
            .collect();
        Peers { number, count: d.views.len(), peers }
    }
}
