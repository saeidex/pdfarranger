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

//! Core PDF operations: merge, split, rotate, reorder, delete and crop pages.

use lopdf::{Document, Object, ObjectId};
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

/// Errors that can occur during PDF operations.
#[derive(Error, Debug)]
pub enum PdfError {
    #[error("lopdf error: {0}")]
    Lopdf(#[from] lopdf::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("page index out of range: page {page} requested but document has {total} pages")]
    PageOutOfRange { page: usize, total: usize },

    #[error("no pages in document")]
    NoPages,

    #[error("no input files provided")]
    NoInputFiles,

    #[error("invalid rotation angle {0}: must be 0, 90, 180 or 270")]
    InvalidRotation(i64),

    #[error("invalid crop values: {0}")]
    InvalidCrop(String),
}

/// A wrapper around a lopdf [`Document`] that provides higher-level PDF operations.
pub struct PdfDocument {
    doc: Document,
}

impl PdfDocument {
    /// Load a PDF from the given file path.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, PdfError> {
        let doc = Document::load(path)?;
        Ok(Self { doc })
    }

    /// Create a new, empty PDF document.
    pub fn new() -> Self {
        Self {
            doc: Document::with_version("1.5"),
        }
    }

    /// Return the number of pages in this document.
    pub fn page_count(&self) -> usize {
        self.doc.get_pages().len()
    }

    /// Save the document to a file.
    pub fn save<P: AsRef<Path>>(&mut self, path: P) -> Result<(), PdfError> {
        self.doc.save(path)?;
        Ok(())
    }

    /// Return a reference to the inner [`Document`].
    pub fn inner(&self) -> &Document {
        &self.doc
    }

    /// Return a mutable reference to the inner [`Document`].
    pub fn inner_mut(&mut self) -> &mut Document {
        &mut self.doc
    }
}

impl Default for PdfDocument {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Merge ───────────────────────────────────────────────────────────────────

/// Merge all `inputs` into a single PDF and return the merged document.
///
/// Pages are appended in the order they appear across the input files.
///
/// # Errors
///
/// Returns [`PdfError::NoInputFiles`] when `inputs` is empty, or propagates
/// I/O / lopdf errors from loading any input file.
pub fn merge<P: AsRef<Path>>(inputs: &[P]) -> Result<PdfDocument, PdfError> {
    if inputs.is_empty() {
        return Err(PdfError::NoInputFiles);
    }

    let mut docs: Vec<Document> = inputs
        .iter()
        .map(|p| Document::load(p).map_err(PdfError::from))
        .collect::<Result<_, _>>()?;

    // Renumber objects in every document so there are no ID collisions.
    let mut max_id: u32 = 1;
    for doc in &mut docs {
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;
    }

    // Collect all pages and all non-structural objects from every document.
    let mut all_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut all_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    for doc in docs {
        // Separate "Page" objects from everything else.
        for (_page_num, page_id) in doc.get_pages() {
            if let Some(obj) = doc.objects.get(&page_id) {
                all_pages.insert(page_id, obj.clone());
            }
        }

        for (object_id, object) in &doc.objects {
            match object.type_name().unwrap_or(b"") {
                b"Catalog" => {
                    catalog_object = Some((
                        catalog_object.as_ref().map(|(id, _)| *id).unwrap_or(*object_id),
                        object.clone(),
                    ));
                }
                b"Pages" => {
                    if let Ok(dict) = object.as_dict() {
                        let mut dict = dict.clone();
                        if let Some((_, ref existing)) = pages_object {
                            if let Ok(old) = existing.as_dict() {
                                dict.extend(old);
                            }
                        }
                        pages_object = Some((
                            pages_object.as_ref().map(|(id, _)| *id).unwrap_or(*object_id),
                            Object::Dictionary(dict),
                        ));
                    }
                }
                b"Page" => {} // handled above
                _ => {
                    all_objects.insert(*object_id, object.clone());
                }
            }
        }
    }

    let (page_id, pages_obj) = pages_object.ok_or(PdfError::NoPages)?;
    let (catalog_id, catalog_obj) = catalog_object.ok_or(PdfError::NoPages)?;

    let mut output = Document::with_version("1.5");

    // Insert all non-structural objects.
    for (id, obj) in all_objects {
        output.objects.insert(id, obj);
    }

    // Re-parent every page to the unified Pages node.
    for (object_id, object) in &all_pages {
        if let Ok(dict) = object.as_dict() {
            let mut dict = dict.clone();
            dict.set("Parent", page_id);
            output.objects.insert(*object_id, Object::Dictionary(dict));
        }
    }

    // Build the unified Pages object.
    if let Ok(dict) = pages_obj.as_dict() {
        let mut dict = dict.clone();
        dict.set("Count", all_pages.len() as u32);
        dict.set(
            "Kids",
            all_pages
                .keys()
                .map(|&id| Object::Reference(id))
                .collect::<Vec<_>>(),
        );
        output.objects.insert(page_id, Object::Dictionary(dict));
    }

    // Build the Catalog.
    if let Ok(dict) = catalog_obj.as_dict() {
        let mut dict = dict.clone();
        dict.set("Pages", page_id);
        output.objects.insert(catalog_id, Object::Dictionary(dict));
    }

    output.trailer.set("Root", catalog_id);
    output.max_id = output.objects.len() as u32;
    output.renumber_objects();

    Ok(PdfDocument { doc: output })
}

// ─── Split ───────────────────────────────────────────────────────────────────

/// Split a PDF into individual single-page documents.
///
/// Returns one [`PdfDocument`] per page (in order).
///
/// # Errors
///
/// Returns [`PdfError::NoPages`] when the input is empty, or propagates lopdf
/// errors.
pub fn split_into_pages(input: &PdfDocument) -> Result<Vec<PdfDocument>, PdfError> {
    let page_count = input.page_count();
    if page_count == 0 {
        return Err(PdfError::NoPages);
    }

    (0..page_count)
        .map(|i| extract_pages(input, &[i]))
        .collect()
}

/// Extract a subset of pages (0-indexed) from `input` and return a new document.
///
/// Pages are written to the output in the order specified by `page_indices`,
/// so this function can also be used to reorder pages.
///
/// # Errors
///
/// Returns [`PdfError::PageOutOfRange`] if any index is out of bounds.
pub fn extract_pages(input: &PdfDocument, page_indices: &[usize]) -> Result<PdfDocument, PdfError> {
    let total = input.page_count();
    for &idx in page_indices {
        if idx >= total {
            return Err(PdfError::PageOutOfRange {
                page: idx + 1,
                total,
            });
        }
    }

    // lopdf numbers pages from 1
    let one_based: Vec<u32> = page_indices.iter().map(|&i| (i + 1) as u32).collect();

    let mut out = input.doc.clone();
    // Collect all 1-based page numbers, then delete those NOT in our set.
    let all_pages: Vec<u32> = out.get_pages().keys().copied().collect();
    let keep: std::collections::HashSet<u32> = one_based.iter().copied().collect();
    let delete: Vec<u32> = all_pages.into_iter().filter(|p| !keep.contains(p)).collect();
    out.delete_pages(&delete);

    Ok(PdfDocument { doc: out })
}

// ─── Delete ──────────────────────────────────────────────────────────────────

/// Delete pages at the given 0-based `page_indices` from a document.
///
/// Returns a new document with those pages removed.
///
/// # Errors
///
/// Returns [`PdfError::PageOutOfRange`] if any index is out of bounds, or
/// [`PdfError::NoPages`] if deleting those pages would leave the document
/// empty.
pub fn delete_pages(input: &PdfDocument, page_indices: &[usize]) -> Result<PdfDocument, PdfError> {
    let total = input.page_count();
    for &idx in page_indices {
        if idx >= total {
            return Err(PdfError::PageOutOfRange {
                page: idx + 1,
                total,
            });
        }
    }

    let delete_set: std::collections::HashSet<usize> =
        page_indices.iter().copied().collect();
    let remaining: Vec<usize> = (0..total)
        .filter(|i| !delete_set.contains(i))
        .collect();

    if remaining.is_empty() {
        return Err(PdfError::NoPages);
    }

    extract_pages(input, &remaining)
}

// ─── Reorder ─────────────────────────────────────────────────────────────────

/// Reorder the pages of `input` according to `new_order`.
///
/// `new_order` is a slice of 0-based page indices that specifies the desired
/// order.  Every page index in `[0, page_count)` must appear exactly once.
///
/// # Errors
///
/// Returns [`PdfError::PageOutOfRange`] if any index is out of bounds.
pub fn reorder_pages(input: &PdfDocument, new_order: &[usize]) -> Result<PdfDocument, PdfError> {
    extract_pages(input, new_order)
}

// ─── Rotate ──────────────────────────────────────────────────────────────────

/// Rotate specific pages in a document by `degrees` (must be 0, 90, 180 or 270).
///
/// `page_indices` is a list of 0-based page numbers to rotate.  Pass an empty
/// slice to rotate *all* pages.
///
/// Returns a new document with the rotations applied.
///
/// # Errors
///
/// Returns [`PdfError::InvalidRotation`] for an unsupported angle, or
/// [`PdfError::PageOutOfRange`] for an out-of-bounds index.
pub fn rotate_pages(
    input: &PdfDocument,
    page_indices: &[usize],
    degrees: i64,
) -> Result<PdfDocument, PdfError> {
    if !matches!(degrees, 0 | 90 | 180 | 270) {
        return Err(PdfError::InvalidRotation(degrees));
    }

    let total = input.page_count();

    // An empty slice means "rotate all pages".
    let indices: Vec<usize> = if page_indices.is_empty() {
        (0..total).collect()
    } else {
        page_indices.to_vec()
    };

    for &idx in &indices {
        if idx >= total {
            return Err(PdfError::PageOutOfRange {
                page: idx + 1,
                total,
            });
        }
    }

    let mut out = input.doc.clone();
    let page_map: BTreeMap<u32, ObjectId> = out.get_pages();

    for idx in indices {
        // lopdf page numbers are 1-based
        let page_num = (idx + 1) as u32;
        if let Some(&page_id) = page_map.get(&page_num) {
            if let Ok(Object::Dictionary(ref mut dict)) = out.get_object_mut(page_id) {
                let existing: i64 = dict
                    .get(b"Rotate")
                    .and_then(|o| o.as_i64())
                    .unwrap_or(0);
                let new_rotation = (existing + degrees).rem_euclid(360);
                dict.set("Rotate", Object::Integer(new_rotation));
            }
        }
    }

    Ok(PdfDocument { doc: out })
}

// ─── Crop ────────────────────────────────────────────────────────────────────

/// Crop margins expressed as fractions of the page dimension in the range
/// `[0.0, 1.0)`.  All four values must be non-negative and `left + right < 1`
/// and `top + bottom < 1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropMargins {
    /// Fraction to remove from the left edge.
    pub left: f64,
    /// Fraction to remove from the right edge.
    pub right: f64,
    /// Fraction to remove from the top edge.
    pub top: f64,
    /// Fraction to remove from the bottom edge.
    pub bottom: f64,
}

impl CropMargins {
    /// Create new crop margins, validating that all values are in `[0.0, 1.0)`
    /// and that opposite edges do not overlap.
    pub fn new(left: f64, right: f64, top: f64, bottom: f64) -> Result<Self, PdfError> {
        for (name, v) in [("left", left), ("right", right), ("top", top), ("bottom", bottom)] {
            if !(0.0..1.0).contains(&v) {
                return Err(PdfError::InvalidCrop(format!(
                    "{name} margin {v} is not in [0, 1)"
                )));
            }
        }
        if left + right >= 1.0 {
            return Err(PdfError::InvalidCrop(format!(
                "left ({left}) + right ({right}) >= 1.0"
            )));
        }
        if top + bottom >= 1.0 {
            return Err(PdfError::InvalidCrop(format!(
                "top ({top}) + bottom ({bottom}) >= 1.0"
            )));
        }
        Ok(Self { left, right, top, bottom })
    }

    /// Return `CropMargins` with all margins set to zero (no cropping).
    pub fn none() -> Self {
        Self { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 }
    }
}

/// Crop specific pages of `input` by setting their `MediaBox`.
///
/// `page_indices` is a list of 0-based page numbers to crop.  Pass an empty
/// slice to crop *all* pages.
///
/// Returns a new document with the `MediaBox` updated on the selected pages.
///
/// # Errors
///
/// Returns [`PdfError::PageOutOfRange`] for an out-of-bounds index.
pub fn crop_pages(
    input: &PdfDocument,
    page_indices: &[usize],
    margins: &CropMargins,
) -> Result<PdfDocument, PdfError> {
    let total = input.page_count();

    let indices: Vec<usize> = if page_indices.is_empty() {
        (0..total).collect()
    } else {
        page_indices.to_vec()
    };

    for &idx in &indices {
        if idx >= total {
            return Err(PdfError::PageOutOfRange {
                page: idx + 1,
                total,
            });
        }
    }

    let mut out = input.doc.clone();
    let page_map: BTreeMap<u32, ObjectId> = out.get_pages();

    for idx in indices {
        let page_num = (idx + 1) as u32;
        if let Some(&page_id) = page_map.get(&page_num) {
            // Resolve the page dictionary, following indirect references.
            let media_box = {
                let page_obj = out.get_object(page_id)?;
                get_media_box(page_obj, &out)
            };

            if let Some([x0, y0, x1, y1]) = media_box {
                let width = x1 - x0;
                let height = y1 - y0;

                let new_x0 = x0 + margins.left * width;
                let new_x1 = x1 - margins.right * width;
                // PDF Y-axis grows upward: "top" in visual terms = high Y values.
                let new_y0 = y0 + margins.bottom * height;
                let new_y1 = y1 - margins.top * height;

                let new_box = Object::Array(vec![
                    Object::Real(new_x0 as f32),
                    Object::Real(new_y0 as f32),
                    Object::Real(new_x1 as f32),
                    Object::Real(new_y1 as f32),
                ]);

                if let Ok(Object::Dictionary(ref mut dict)) = out.get_object_mut(page_id) {
                    dict.set("MediaBox", new_box);
                }
            }
        }
    }

    Ok(PdfDocument { doc: out })
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Try to extract the `[x0, y0, x1, y1]` coordinates from a page's `MediaBox`.
fn get_media_box(page_obj: &Object, doc: &Document) -> Option<[f64; 4]> {
    let dict = match page_obj {
        Object::Dictionary(d) => d,
        _ => return None,
    };

    let mb = dict.get(b"MediaBox").ok()?;
    let arr = match mb {
        Object::Array(a) => a,
        Object::Reference(r) => {
            if let Ok(Object::Array(a)) = doc.get_object(*r) {
                return array_to_rect(a);
            }
            return None;
        }
        _ => return None,
    };

    array_to_rect(arr)
}

fn array_to_rect(arr: &[Object]) -> Option<[f64; 4]> {
    if arr.len() < 4 {
        return None;
    }
    let nums: Option<Vec<f64>> = arr
        .iter()
        .take(4)
        .map(|o| match o {
            Object::Integer(i) => Some(*i as f64),
            Object::Real(r) => Some(*r as f64),
            _ => None,
        })
        .collect();
    nums.map(|v| [v[0], v[1], v[2], v[3]])
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Dictionary, Stream};

    /// Build a valid multi-page PDF document with `n_pages` A4 pages entirely
    /// in memory using lopdf primitives.  Every page gets a distinct MediaBox
    /// so crop tests can verify the box values change.
    fn make_doc(n_pages: usize) -> PdfDocument {
        assert!(n_pages > 0);
        let mut doc = Document::with_version("1.5");

        let pages_id = doc.new_object_id();

        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("ET", vec![]),
            ],
        };
        let encoded = content.encode().unwrap();

        let mut kids: Vec<Object> = Vec::with_capacity(n_pages);
        for _ in 0..n_pages {
            let stream_id =
                doc.add_object(Stream::new(Dictionary::new(), encoded.clone()));
            let page_id = doc.add_object(dictionary! {
                "Type"     => "Page",
                "Parent"   => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                "Contents" => stream_id,
            });
            kids.push(Object::Reference(page_id));
        }

        let pages = dictionary! {
            "Type"  => "Pages",
            "Kids"  => kids,
            "Count" => n_pages as i64,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let catalog_id = doc.add_object(dictionary! {
            "Type"  => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        PdfDocument { doc }
    }

    /// Persist `doc` to a temp file, returning the path (kept alive by the
    /// `TempDir` handle which the caller must hold).
    fn save_to_tempfile(doc: &mut PdfDocument) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.pdf");
        doc.save(&path).unwrap();
        (dir, path)
    }

    // ── CropMargins ──────────────────────────────────────────────────────────

    #[test]
    fn crop_margins_valid() {
        let m = CropMargins::new(0.1, 0.1, 0.1, 0.1).unwrap();
        assert_eq!(m.left, 0.1);
    }

    #[test]
    fn crop_margins_none_is_zero() {
        let m = CropMargins::none();
        assert_eq!(m.left, 0.0);
        assert_eq!(m.right, 0.0);
        assert_eq!(m.top, 0.0);
        assert_eq!(m.bottom, 0.0);
    }

    #[test]
    fn crop_margins_overlap_is_rejected() {
        assert!(CropMargins::new(0.6, 0.6, 0.0, 0.0).is_err());
    }

    #[test]
    fn crop_margins_negative_is_rejected() {
        assert!(CropMargins::new(-0.1, 0.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn crop_margins_one_is_rejected() {
        assert!(CropMargins::new(1.0, 0.0, 0.0, 0.0).is_err());
    }

    // ── PdfDocument ──────────────────────────────────────────────────────────

    #[test]
    fn make_doc_has_correct_page_count() {
        for n in [1, 2, 5] {
            let doc = make_doc(n);
            assert_eq!(doc.page_count(), n, "expected {n} pages");
        }
    }

    #[test]
    fn new_document_has_zero_pages() {
        let doc = PdfDocument::new();
        assert_eq!(doc.page_count(), 0);
    }

    // ── merge ────────────────────────────────────────────────────────────────

    #[test]
    fn merge_empty_input_fails() {
        let result = merge::<&str>(&[]);
        assert!(matches!(result, Err(PdfError::NoInputFiles)));
    }

    #[test]
    fn merge_single_file() {
        let mut doc = make_doc(2);
        let (_dir, path) = save_to_tempfile(&mut doc);
        let merged = merge(&[&path]).unwrap();
        assert_eq!(merged.page_count(), 2);
    }

    #[test]
    fn merge_two_files() {
        let mut doc1 = make_doc(2);
        let mut doc2 = make_doc(3);
        let (_d1, p1) = save_to_tempfile(&mut doc1);
        let (_d2, p2) = save_to_tempfile(&mut doc2);
        let merged = merge(&[&p1, &p2]).unwrap();
        assert_eq!(merged.page_count(), 5);
    }

    // ── split_into_pages ─────────────────────────────────────────────────────

    #[test]
    fn split_into_pages_count() {
        let doc = make_doc(3);
        let pages = split_into_pages(&doc).unwrap();
        assert_eq!(pages.len(), 3);
        for p in &pages {
            assert_eq!(p.page_count(), 1);
        }
    }

    // ── extract_pages ────────────────────────────────────────────────────────

    #[test]
    fn extract_first_page() {
        let doc = make_doc(3);
        let extracted = extract_pages(&doc, &[0]).unwrap();
        assert_eq!(extracted.page_count(), 1);
    }

    #[test]
    fn extract_multiple_pages() {
        let doc = make_doc(5);
        let extracted = extract_pages(&doc, &[0, 2, 4]).unwrap();
        assert_eq!(extracted.page_count(), 3);
    }

    #[test]
    fn extract_out_of_range() {
        let doc = make_doc(2);
        let n = doc.page_count();
        assert!(matches!(
            extract_pages(&doc, &[n]),
            Err(PdfError::PageOutOfRange { .. })
        ));
    }

    // ── delete_pages ─────────────────────────────────────────────────────────

    #[test]
    fn delete_first_page() {
        let doc = make_doc(3);
        let result = delete_pages(&doc, &[0]).unwrap();
        assert_eq!(result.page_count(), 2);
    }

    #[test]
    fn delete_all_pages_fails() {
        let doc = make_doc(2);
        let all: Vec<usize> = (0..doc.page_count()).collect();
        assert!(matches!(delete_pages(&doc, &all), Err(PdfError::NoPages)));
    }

    #[test]
    fn delete_out_of_range() {
        let doc = make_doc(2);
        let n = doc.page_count();
        assert!(matches!(
            delete_pages(&doc, &[n]),
            Err(PdfError::PageOutOfRange { .. })
        ));
    }

    // ── reorder_pages ────────────────────────────────────────────────────────

    #[test]
    fn reorder_identity() {
        let doc = make_doc(3);
        let n = doc.page_count();
        let order: Vec<usize> = (0..n).collect();
        let reordered = reorder_pages(&doc, &order).unwrap();
        assert_eq!(reordered.page_count(), n);
    }

    #[test]
    fn reorder_reverse() {
        let doc = make_doc(3);
        let n = doc.page_count();
        let order: Vec<usize> = (0..n).rev().collect();
        let reordered = reorder_pages(&doc, &order).unwrap();
        assert_eq!(reordered.page_count(), n);
    }

    // ── rotate_pages ─────────────────────────────────────────────────────────

    #[test]
    fn rotate_invalid_angle() {
        let doc = make_doc(1);
        assert!(matches!(
            rotate_pages(&doc, &[], 45),
            Err(PdfError::InvalidRotation(45))
        ));
    }

    #[test]
    fn rotate_all_pages_90() {
        let doc = make_doc(2);
        let rotated = rotate_pages(&doc, &[], 90).unwrap();
        assert_eq!(rotated.page_count(), doc.page_count());
    }

    #[test]
    fn rotate_specific_page() {
        let doc = make_doc(3);
        let rotated = rotate_pages(&doc, &[0], 180).unwrap();
        assert_eq!(rotated.page_count(), doc.page_count());
    }

    #[test]
    fn rotate_out_of_range() {
        let doc = make_doc(2);
        let n = doc.page_count();
        assert!(matches!(
            rotate_pages(&doc, &[n], 90),
            Err(PdfError::PageOutOfRange { .. })
        ));
    }

    #[test]
    fn rotate_360_is_noop() {
        let doc = make_doc(2);
        let rotated = rotate_pages(&doc, &[], 0).unwrap();
        assert_eq!(rotated.page_count(), doc.page_count());
    }

    // ── crop_pages ───────────────────────────────────────────────────────────

    #[test]
    fn crop_zero_margins_is_noop() {
        let doc = make_doc(2);
        let margins = CropMargins::none();
        let cropped = crop_pages(&doc, &[], &margins).unwrap();
        assert_eq!(cropped.page_count(), doc.page_count());
    }

    #[test]
    fn crop_with_margins() {
        let doc = make_doc(2);
        let margins = CropMargins::new(0.1, 0.1, 0.1, 0.1).unwrap();
        let cropped = crop_pages(&doc, &[], &margins).unwrap();
        assert_eq!(cropped.page_count(), doc.page_count());
    }

    #[test]
    fn crop_out_of_range() {
        let doc = make_doc(2);
        let n = doc.page_count();
        let margins = CropMargins::none();
        assert!(matches!(
            crop_pages(&doc, &[n], &margins),
            Err(PdfError::PageOutOfRange { .. })
        ));
    }

    // ── save / round-trip ────────────────────────────────────────────────────

    #[test]
    fn save_and_reload() {
        let doc = make_doc(2);
        let mut rotated = rotate_pages(&doc, &[], 90).unwrap();
        let (_dir, path) = save_to_tempfile(&mut rotated);
        let reloaded = PdfDocument::load(&path).unwrap();
        assert_eq!(reloaded.page_count(), doc.page_count());
    }

    #[test]
    fn merge_save_and_reload() {
        let mut doc1 = make_doc(1);
        let mut doc2 = make_doc(2);
        let (_d1, p1) = save_to_tempfile(&mut doc1);
        let (_d2, p2) = save_to_tempfile(&mut doc2);
        let mut merged = merge(&[&p1, &p2]).unwrap();
        let (_dir, out) = save_to_tempfile(&mut merged);
        let reloaded = PdfDocument::load(&out).unwrap();
        assert_eq!(reloaded.page_count(), 3);
    }
}
