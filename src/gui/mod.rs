// Copyright (C) 2024 pdfarranger contributors
//
// pdfarranger is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along
// with this program; if not, write to the Free Software Foundation, Inc.,
// 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA.

//! GTK3 GUI for PDF Arranger.
//!
//! # Architecture
//!
//! The GUI is built with `gtk-rs` (GTK 3 bindings).  Application state is
//! held inside a reference-counted, interior-mutable [`AppState`] struct that
//! is shared between GTK signal handlers.
//!
//! ## Window layout
//!
//! ```text
//! ┌────────────────────────────────────────────────────────┐
//! │  Menu bar  (File | Edit | Help)                        │
//! ├────────────────────────────────────────────────────────┤
//! │  Tool bar  [Open] [Add] [Save] | [↺] [↻] [⌫] [↑] [↓] │
//! ├────────────────────────────────────────────────────────┤
//! │                                                        │
//! │  Page list  (scrollable, multi-select)                 │
//! │  ┌─────────────────────────────────────────────────┐   │
//! │  │ # │ File          │ Page │ Rotation              │   │
//! │  ├───┼───────────────┼──────┼───────────────────────┤   │
//! │  │ 1 │ document.pdf  │  1   │    0°                 │   │
//! │  │ 2 │ document.pdf  │  2   │    0°                 │   │
//! │  └─────────────────────────────────────────────────┘   │
//! │                                                        │
//! ├────────────────────────────────────────────────────────┤
//! │  Status bar  "3 pages loaded"                          │
//! └────────────────────────────────────────────────────────┘
//! ```

pub mod app;

pub use app::run;
