//! SpeedSheet core: a fast, UI-independent spreadsheet engine.
//!
//! Everything a front-end needs lives here: reading and writing files, the cell model,
//! formulas, sorting, filtering, searching and undo/redo. The Tauri app is a thin shell
//! around this crate, so it can be swapped for a native UI later.

pub mod fill;
pub mod filter;
pub mod formula;
pub mod input;
pub mod io;
pub mod layout;
pub mod numfmt;
pub mod ops;
pub mod shared;
pub mod sheet;
pub mod style;
pub mod value;
pub mod workbook;

pub use filter::{Filter, FilterCond, FilterOp};
pub use sheet::{col_name, Cell, Sheet};
pub use value::{CellError, Value};
pub use workbook::{Change, DefinedName, FileFormat, Match, VbaProject, Workbook};

/// Excel's limits.
pub const MAX_ROWS: usize = 1_048_576;
pub const MAX_COLS: usize = 16_384;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not read spreadsheet: {0}")]
    Read(String),
    #[error("could not write spreadsheet: {0}")]
    Write(String),
    #[error("unsupported file type: {0}")]
    Unsupported(String),
    #[error("sheet {0} does not exist")]
    NoSuchSheet(usize),
    #[error("invalid sheet name: {0}")]
    InvalidName(String),
    #[error("a workbook needs at least one sheet")]
    LastSheet,
    /// Saving over the opened file would remove these features (see `Workbook::losses`).
    #[error("saving over the original file would remove: {}", .0.join(", "))]
    WouldLoseData(Vec<String>),
}

pub type Result<T> = std::result::Result<T, Error>;
