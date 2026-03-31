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

//! PDF Arranger — command-line interface.
//!
//! # Subcommands
//!
//! | Command    | Description                                          |
//! |------------|------------------------------------------------------|
//! | `merge`    | Merge multiple PDF files into one                    |
//! | `split`    | Split a PDF into individual single-page files        |
//! | `rotate`   | Rotate selected (or all) pages                       |
//! | `delete`   | Remove selected pages from a PDF                    |
//! | `reorder`  | Reorder pages in a new sequence                      |
//! | `crop`     | Crop pages by removing fractional margins            |
//! | `info`     | Print the number of pages in a PDF                  |

use clap::{Parser, Subcommand};
use pdfarranger::core::{
    crop_pages, delete_pages, merge, reorder_pages, rotate_pages, split_into_pages, CropMargins,
    PdfDocument, PdfError,
};
use std::path::PathBuf;
use std::process;

// ─── CLI definition ──────────────────────────────────────────────────────────

/// Merge, split, rotate, crop and rearrange PDF pages.
#[derive(Parser, Debug)]
#[command(name = "pdfarranger", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Merge multiple PDF files into a single output file.
    Merge {
        /// Input PDF files (two or more).
        #[arg(required = true, num_args = 2..)]
        inputs: Vec<PathBuf>,

        /// Output PDF file.
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Split a PDF into one file per page.
    ///
    /// Output files are named `<stem>-001.pdf`, `<stem>-002.pdf`, …
    Split {
        /// Input PDF file.
        input: PathBuf,

        /// Directory where per-page PDFs are written (defaults to current directory).
        #[arg(short, long, default_value = ".")]
        outdir: PathBuf,
    },

    /// Rotate pages in a PDF.
    Rotate {
        /// Input PDF file.
        input: PathBuf,

        /// Clockwise rotation in degrees (0, 90, 180 or 270).
        #[arg(short, long)]
        degrees: i64,

        /// 1-based page numbers to rotate; omit to rotate all pages.
        #[arg(short, long, num_args = 0..)]
        pages: Vec<usize>,

        /// Output PDF file.
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Delete pages from a PDF.
    Delete {
        /// Input PDF file.
        input: PathBuf,

        /// 1-based page numbers to delete.
        #[arg(required = true, num_args = 1..)]
        pages: Vec<usize>,

        /// Output PDF file.
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Reorder pages in a PDF.
    Reorder {
        /// Input PDF file.
        input: PathBuf,

        /// New page order as 1-based page numbers (e.g. `3 1 2`).
        #[arg(required = true, num_args = 1..)]
        order: Vec<usize>,

        /// Output PDF file.
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Crop pages by removing fractional margins (values in [0, 1)).
    Crop {
        /// Input PDF file.
        input: PathBuf,

        /// Fraction to remove from the left edge.
        #[arg(long, default_value_t = 0.0)]
        left: f64,

        /// Fraction to remove from the right edge.
        #[arg(long, default_value_t = 0.0)]
        right: f64,

        /// Fraction to remove from the top edge.
        #[arg(long, default_value_t = 0.0)]
        top: f64,

        /// Fraction to remove from the bottom edge.
        #[arg(long, default_value_t = 0.0)]
        bottom: f64,

        /// 1-based page numbers to crop; omit to crop all pages.
        #[arg(short, long, num_args = 0..)]
        pages: Vec<usize>,

        /// Output PDF file.
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Print information about a PDF file.
    Info {
        /// PDF file to inspect.
        input: PathBuf,
    },
}

// ─── main ────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli.command) {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run(command: Command) -> Result<(), PdfError> {
    match command {
        Command::Merge { inputs, output } => {
            println!(
                "Merging {} file(s) into '{}'…",
                inputs.len(),
                output.display()
            );
            let mut merged = merge(&inputs)?;
            println!("  {} pages in output", merged.page_count());
            merged.save(&output)?;
            println!("Done.");
        }

        Command::Split { input, outdir } => {
            let doc = PdfDocument::load(&input)?;
            let stem = input
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let pages = split_into_pages(&doc)?;
            let n = pages.len();
            let digits = n.to_string().len().max(3);
            println!("Splitting '{}' into {n} page(s)…", input.display());
            std::fs::create_dir_all(&outdir)?;
            for (i, mut page_doc) in pages.into_iter().enumerate() {
                let filename = format!("{stem}-{:0>digits$}.pdf", i + 1);
                let out_path = outdir.join(&filename);
                page_doc.save(&out_path)?;
                println!("  wrote '{}'", out_path.display());
            }
            println!("Done.");
        }

        Command::Rotate {
            input,
            degrees,
            pages,
            output,
        } => {
            let doc = PdfDocument::load(&input)?;
            // Convert 1-based CLI page numbers to 0-based indices.
            let indices: Vec<usize> = pages.iter().map(|&p| p.saturating_sub(1)).collect();
            let rotated = rotate_pages(&doc, &indices, degrees)?;
            let mut rotated = rotated;
            rotated.save(&output)?;
            if pages.is_empty() {
                println!(
                    "Rotated all pages of '{}' by {degrees}° → '{}'",
                    input.display(),
                    output.display()
                );
            } else {
                println!(
                    "Rotated {} page(s) of '{}' by {degrees}° → '{}'",
                    pages.len(),
                    input.display(),
                    output.display()
                );
            }
        }

        Command::Delete {
            input,
            pages,
            output,
        } => {
            let doc = PdfDocument::load(&input)?;
            let indices: Vec<usize> = pages.iter().map(|&p| p.saturating_sub(1)).collect();
            let result = delete_pages(&doc, &indices)?;
            let mut result = result;
            result.save(&output)?;
            println!(
                "Deleted {} page(s) from '{}' → '{}' ({} page(s) remaining)",
                pages.len(),
                input.display(),
                output.display(),
                result.page_count()
            );
        }

        Command::Reorder {
            input,
            order,
            output,
        } => {
            let doc = PdfDocument::load(&input)?;
            let indices: Vec<usize> = order.iter().map(|&p| p.saturating_sub(1)).collect();
            let reordered = reorder_pages(&doc, &indices)?;
            let mut reordered = reordered;
            reordered.save(&output)?;
            println!(
                "Reordered '{}' → '{}' ({} page(s))",
                input.display(),
                output.display(),
                reordered.page_count()
            );
        }

        Command::Crop {
            input,
            left,
            right,
            top,
            bottom,
            pages,
            output,
        } => {
            let doc = PdfDocument::load(&input)?;
            let margins = CropMargins::new(left, right, top, bottom)?;
            let indices: Vec<usize> = pages.iter().map(|&p| p.saturating_sub(1)).collect();
            let cropped = crop_pages(&doc, &indices, &margins)?;
            let mut cropped = cropped;
            cropped.save(&output)?;
            if pages.is_empty() {
                println!(
                    "Cropped all pages of '{}' → '{}'",
                    input.display(),
                    output.display()
                );
            } else {
                println!(
                    "Cropped {} page(s) of '{}' → '{}'",
                    pages.len(),
                    input.display(),
                    output.display()
                );
            }
        }

        Command::Info { input } => {
            let doc = PdfDocument::load(&input)?;
            println!("'{}': {} page(s)", input.display(), doc.page_count());
        }
    }

    Ok(())
}
