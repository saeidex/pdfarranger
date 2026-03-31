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

//! Entry point for the `pdfarranger-gui` binary.
//!
//! Usage:
//! ```text
//! pdfarranger-gui [PDF_FILE...]
//! ```
//!
//! Any PDF files passed on the command line are pre-loaded into the application
//! on start-up.

use std::path::PathBuf;

fn main() {
    // Collect optional PDF file arguments (skip argv[0]).
    let files: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect();

    let exit_code = pdfarranger::gui::run(files);
    std::process::exit(exit_code.into());
}
