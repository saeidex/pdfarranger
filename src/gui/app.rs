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

//! GTK3 application state and UI construction.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gio::prelude::*;
use glib::clone;
use gtk::prelude::*;
use gtk::{
    AboutDialog, Application, ApplicationWindow, Box as GtkBox, Button, CellRendererText,
    FileChooserAction, FileChooserDialog, FileFilter, HeaderBar, ListStore,
    MessageDialog, MessageType, Orientation, PolicyType, ResponseType, ScrolledWindow,
    SelectionMode, Separator, SortColumn, SortType, Statusbar, TreeView, TreeViewColumn,
    TreeViewColumnSizing,
};

// ─── Column indices in the ListStore ─────────────────────────────────────────

const COL_INDEX: u32 = 0; // display order (1-based)
const COL_FILENAME: u32 = 1; // file name (no path)
const COL_PAGE: u32 = 2; // 1-based page number within source file
const COL_ROTATION: u32 = 3; // current rotation in degrees
const COL_SOURCE_PATH: u32 = 4; // full source file path (hidden)
const COL_SOURCE_PAGE: u32 = 5; // original 0-based page index (hidden)

// ─── A single page entry held in memory ──────────────────────────────────────

#[derive(Debug, Clone)]
struct PageEntry {
    /// Full path of the source PDF.
    source_path: PathBuf,
    /// 0-based index within the source PDF.
    source_page: usize,
    /// Accumulated rotation (0 / 90 / 180 / 270).
    rotation: i64,
}

// ─── Shared application state ─────────────────────────────────────────────────

struct AppState {
    /// Ordered list of page entries (the "working set").
    pages: Vec<PageEntry>,
    /// The GTK list store backing the TreeView.
    store: ListStore,
    /// Status bar context id.
    status_ctx: u32,
    /// Status bar widget.
    statusbar: Statusbar,
}

impl AppState {
    fn new(store: ListStore, statusbar: Statusbar) -> Self {
        let status_ctx = statusbar.context_id("main");
        Self {
            pages: Vec::new(),
            store,
            status_ctx,
            statusbar,
        }
    }

    /// Refresh the entire ListStore from `self.pages`.
    fn refresh_store(&self) {
        self.store.clear();
        for (i, entry) in self.pages.iter().enumerate() {
            let filename = entry
                .source_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.store.insert_with_values(
                None,
                &[
                    (COL_INDEX, &((i + 1) as u32)),
                    (COL_FILENAME, &filename),
                    (COL_PAGE, &((entry.source_page + 1) as u32)),
                    (COL_ROTATION, &(entry.rotation as i32)),
                    (COL_SOURCE_PATH, &entry.source_path.to_string_lossy().as_ref()),
                    (COL_SOURCE_PAGE, &(entry.source_page as u32)),
                ],
            );
        }
        self.update_status();
    }

    fn update_status(&self) {
        let msg = if self.pages.is_empty() {
            "No pages loaded".to_owned()
        } else {
            format!("{} page(s) loaded", self.pages.len())
        };
        self.statusbar.pop(self.status_ctx);
        self.statusbar.push(self.status_ctx, &msg);
    }

    /// Add all pages from the given PDF file.
    fn add_file(&mut self, path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        use crate::core::PdfDocument;
        let doc = PdfDocument::load(&path)?;
        let n = doc.page_count();
        for page_idx in 0..n {
            self.pages.push(PageEntry {
                source_path: path.clone(),
                source_page: page_idx,
                rotation: 0,
            });
        }
        Ok(())
    }

    /// Return the 0-based indices of currently selected rows.
    fn selected_indices(tree_view: &TreeView) -> Vec<usize> {
        let selection = tree_view.selection();
        let (selected_paths, model) = selection.selected_rows();
        selected_paths
            .iter()
            .filter_map(|path| {
                model.iter(path).and_then(|iter| {
                    model
                        .value(&iter, COL_INDEX as i32)
                        .get::<u32>()
                        .ok()
                        .map(|idx| (idx as usize).saturating_sub(1))
                })
            })
            .collect()
    }

    /// Rotate selected pages by `delta` degrees (positive = clockwise).
    fn rotate_selected(&mut self, tree_view: &TreeView, delta: i64) {
        let indices = Self::selected_indices(tree_view);
        if indices.is_empty() {
            return;
        }
        for &idx in &indices {
            if let Some(entry) = self.pages.get_mut(idx) {
                entry.rotation = (entry.rotation + delta).rem_euclid(360);
            }
        }
        self.refresh_store();
        // Restore selection after refresh.
        self.reselect(tree_view, &indices);
    }

    /// Delete selected pages.
    fn delete_selected(&mut self, tree_view: &TreeView) {
        let mut indices = Self::selected_indices(tree_view);
        if indices.is_empty() {
            return;
        }
        indices.sort_unstable_by(|a, b| b.cmp(a)); // descending
        for idx in indices {
            if idx < self.pages.len() {
                self.pages.remove(idx);
            }
        }
        self.refresh_store();
    }

    /// Move selected pages up by one position.
    fn move_up(&mut self, tree_view: &TreeView) {
        let mut indices = Self::selected_indices(tree_view);
        indices.sort_unstable();
        if indices.is_empty() || indices[0] == 0 {
            return;
        }
        for &idx in &indices {
            self.pages.swap(idx - 1, idx);
        }
        let new_indices: Vec<usize> = indices.iter().map(|&i| i - 1).collect();
        self.refresh_store();
        self.reselect(tree_view, &new_indices);
    }

    /// Move selected pages down by one position.
    fn move_down(&mut self, tree_view: &TreeView) {
        let mut indices = Self::selected_indices(tree_view);
        indices.sort_unstable_by(|a, b| b.cmp(a));
        if indices.is_empty() || *indices.first().unwrap() >= self.pages.len() - 1 {
            return;
        }
        for &idx in &indices {
            self.pages.swap(idx, idx + 1);
        }
        let new_indices: Vec<usize> = indices.iter().map(|&i| i + 1).collect();
        self.refresh_store();
        self.reselect(tree_view, &new_indices);
    }

    /// Re-select rows after a store refresh.
    fn reselect(&self, tree_view: &TreeView, indices: &[usize]) {
        let selection = tree_view.selection();
        selection.unselect_all();
        for &idx in indices {
            let path = gtk::TreePath::from_indicesv(&[idx as i32]);
            selection.select_path(&path);
        }
        // Scroll to first selected row.
        if let Some(&first) = indices.iter().min() {
            let path = gtk::TreePath::from_indicesv(&[first as i32]);
            tree_view.scroll_to_cell(
                Some(&path),
                None::<&TreeViewColumn>,
                false,
                0.0,
                0.0,
            );
        }
    }

    /// Export all pages (respecting current order and rotations) to `output`.
    fn export(&mut self, output: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        use crate::core::{extract_pages, merge, rotate_pages, PdfDocument};
        use std::collections::HashMap;

        if self.pages.is_empty() {
            return Err("No pages to save".into());
        }

        // Group unique source files and load them.
        let mut source_docs: HashMap<String, PdfDocument> = HashMap::new();
        for entry in &self.pages {
            let key = entry.source_path.to_string_lossy().into_owned();
            if let std::collections::hash_map::Entry::Vacant(e) = source_docs.entry(key) {
                e.insert(PdfDocument::load(&entry.source_path)?);
            }
        }

        // For each page slot, extract + rotate and save as a temp segment,
        // then merge all segments together.
        let tmp_dir = std::env::temp_dir().join(format!("pdfarranger_export_{}", std::process::id()));
        std::fs::create_dir_all(&tmp_dir)?;

        let mut segment_paths: Vec<PathBuf> = Vec::new();

        for (slot, entry) in self.pages.iter().enumerate() {
            let key = entry.source_path.to_string_lossy().into_owned();
            let src = source_docs.get(&key).unwrap();

            // Extract single page.
            let single = extract_pages(src, &[entry.source_page])?;
            // Apply rotation if any.
            let rotated = if entry.rotation != 0 {
                rotate_pages(&single, &[], entry.rotation)?
            } else {
                single
            };

            let seg_path = tmp_dir.join(format!("seg_{slot:06}.pdf"));
            let mut rotated = rotated;
            rotated.save(&seg_path)?;
            segment_paths.push(seg_path);
        }

        // Merge all segments.
        let result = merge(&segment_paths);

        // Clean up temp dir regardless of outcome.
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let mut merged = result?;
        merged.save(output)?;
        Ok(())
    }
}

// ─── Build the page-list TreeView ────────────────────────────────────────────

fn build_tree_view(store: &ListStore) -> TreeView {
    let tv = TreeView::with_model(store);
    tv.set_headers_visible(true);
    tv.selection().set_mode(SelectionMode::Multiple);
    tv.set_reorderable(false);

    let make_col = |title: &str, col_id: u32, sortable: bool| {
        let renderer = CellRendererText::new();
        let col = TreeViewColumn::new();
        col.set_title(title);
        col.set_sizing(TreeViewColumnSizing::Autosize);
        col.set_resizable(true);
        gtk::prelude::CellLayoutExt::pack_start(&col, &renderer, true);
        gtk::prelude::CellLayoutExt::add_attribute(&col, &renderer, "text", col_id as i32);
        if sortable {
            col.set_sort_column_id(col_id as i32);
        }
        col
    };

    tv.append_column(&make_col("#", COL_INDEX, true));
    tv.append_column(&make_col("File", COL_FILENAME, true));
    tv.append_column(&make_col("Page", COL_PAGE, false));
    tv.append_column(&make_col("Rotation", COL_ROTATION, false));

    tv
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Launch the GTK3 GUI, blocking until the window is closed.
///
/// Accepts an optional list of PDF files to pre-load.
pub fn run(initial_files: Vec<PathBuf>) -> glib::ExitCode {
    let app = Application::builder()
        .application_id("com.github.pdfarranger")
        .build();

    app.connect_activate(move |app| {
        build_ui(app, initial_files.clone());
    });

    app.run()
}

// ─── UI construction ─────────────────────────────────────────────────────────

fn build_ui(app: &Application, initial_files: Vec<PathBuf>) {
    // ── List store: COL_INDEX u32, COL_FILENAME str, COL_PAGE u32,
    //               COL_ROTATION i32, COL_SOURCE_PATH str, COL_SOURCE_PAGE u32
    let store = ListStore::new(&[
        u32::static_type(),
        String::static_type(),
        u32::static_type(),
        i32::static_type(),
        String::static_type(),
        u32::static_type(),
    ]);
    store.set_sort_column_id(SortColumn::Index(COL_INDEX), SortType::Ascending);

    // ── Status bar ──────────────────────────────────────────────────────────
    let statusbar = Statusbar::new();

    // ── Shared state ────────────────────────────────────────────────────────
    let state = Rc::new(RefCell::new(AppState::new(store.clone(), statusbar.clone())));

    // ── Tree view ───────────────────────────────────────────────────────────
    let tree_view = build_tree_view(&store);
    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Automatic)
        .expand(true)
        .build();
    scrolled.add(&tree_view);

    // ── Header bar ──────────────────────────────────────────────────────────
    let header = HeaderBar::new();
    header.set_title(Some("PDF Arranger"));
    header.set_show_close_button(true);

    // ── Window ──────────────────────────────────────────────────────────────
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(780)
        .default_height(520)
        .title("PDF Arranger")
        .build();
    window.set_titlebar(Some(&header));

    // ── Menu bar ────────────────────────────────────────────────────────────
    let menubar = build_menu_bar(&window, &tree_view, Rc::clone(&state));

    // ── Toolbar ─────────────────────────────────────────────────────────────
    let toolbar = build_toolbar(&window, &tree_view, Rc::clone(&state));

    // ── Root layout ─────────────────────────────────────────────────────────
    let root = GtkBox::new(Orientation::Vertical, 0);
    root.pack_start(&menubar, false, false, 0);
    root.pack_start(&toolbar, false, false, 0);
    root.pack_start(&scrolled, true, true, 0);
    root.pack_start(&Separator::new(Orientation::Horizontal), false, false, 0);
    root.pack_start(&statusbar, false, false, 0);

    window.add(&root);

    // ── Pre-load files passed on the command line ────────────────────────────
    if !initial_files.is_empty() {
        let mut st = state.borrow_mut();
        for path in &initial_files {
            if let Err(e) = st.add_file(path.clone()) {
                eprintln!("Warning: could not load '{}': {e}", path.display());
            }
        }
        st.refresh_store();
    } else {
        state.borrow().update_status();
    }

    window.show_all();
}

// ─── Toolbar ─────────────────────────────────────────────────────────────────

fn build_toolbar(
    window: &ApplicationWindow,
    tree_view: &TreeView,
    state: Rc<RefCell<AppState>>,
) -> GtkBox {
    let bar = GtkBox::new(Orientation::Horizontal, 4);
    bar.set_margin_start(6);
    bar.set_margin_end(6);
    bar.set_margin_top(4);
    bar.set_margin_bottom(4);

    // ── File buttons ────────────────────────────────────────────────────────
    let btn_open = Button::with_label("Open…");
    btn_open.set_tooltip_text(Some("Open PDF files (replaces current list)"));
    let btn_add = Button::with_label("Add…");
    btn_add.set_tooltip_text(Some("Add PDF files to the end of the list"));
    let btn_save = Button::with_label("Save As…");
    btn_save.set_tooltip_text(Some("Export the current page arrangement to a PDF"));

    let sep1 = Separator::new(Orientation::Vertical);

    // ── Page operation buttons ───────────────────────────────────────────────
    let btn_rot_ccw = Button::with_label("↺ 90°");
    btn_rot_ccw.set_tooltip_text(Some("Rotate selected pages 90° counter-clockwise"));
    let btn_rot_cw = Button::with_label("↻ 90°");
    btn_rot_cw.set_tooltip_text(Some("Rotate selected pages 90° clockwise"));
    let btn_delete = Button::with_label("Delete");
    btn_delete.set_tooltip_text(Some("Delete selected pages"));
    let btn_up = Button::with_label("▲ Move Up");
    btn_up.set_tooltip_text(Some("Move selected pages up"));
    let btn_down = Button::with_label("▼ Move Down");
    btn_down.set_tooltip_text(Some("Move selected pages down"));

    bar.pack_start(&btn_open, false, false, 0);
    bar.pack_start(&btn_add, false, false, 0);
    bar.pack_start(&btn_save, false, false, 0);
    bar.pack_start(&sep1, false, false, 4);
    bar.pack_start(&btn_rot_ccw, false, false, 0);
    bar.pack_start(&btn_rot_cw, false, false, 0);
    bar.pack_start(&btn_delete, false, false, 0);
    bar.pack_start(&btn_up, false, false, 0);
    bar.pack_start(&btn_down, false, false, 0);

    // ── Signal handlers ──────────────────────────────────────────────────────

    // Open (replaces current list)
    btn_open.connect_clicked(clone!(@strong window, @strong state, @strong tree_view => move |_| {
        open_files_dialog(&window, &state, &tree_view, true);
    }));

    // Add (appends)
    btn_add.connect_clicked(clone!(@strong window, @strong state, @strong tree_view => move |_| {
        open_files_dialog(&window, &state, &tree_view, false);
    }));

    // Save As
    btn_save.connect_clicked(clone!(@strong window, @strong state => move |_| {
        save_file_dialog(&window, &state);
    }));

    // Rotate CCW = -90° cumulative
    btn_rot_ccw.connect_clicked(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().rotate_selected(&tree_view, 270);
    }));

    // Rotate CW = +90°
    btn_rot_cw.connect_clicked(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().rotate_selected(&tree_view, 90);
    }));

    // Delete
    btn_delete.connect_clicked(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().delete_selected(&tree_view);
    }));

    // Move up
    btn_up.connect_clicked(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().move_up(&tree_view);
    }));

    // Move down
    btn_down.connect_clicked(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().move_down(&tree_view);
    }));

    bar
}

// ─── Menu bar ────────────────────────────────────────────────────────────────

fn build_menu_bar(
    window: &ApplicationWindow,
    tree_view: &TreeView,
    state: Rc<RefCell<AppState>>,
) -> gtk::MenuBar {
    let menubar = gtk::MenuBar::new();

    // ── File menu ───────────────────────────────────────────────────────────
    let file_menu = gtk::Menu::new();
    let item_file = gtk::MenuItem::with_label("_File");
    item_file.set_use_underline(true);
    item_file.set_submenu(Some(&file_menu));

    let item_open = gtk::MenuItem::with_label("_Open…");
    item_open.set_use_underline(true);
    let item_add = gtk::MenuItem::with_label("_Add Files…");
    item_add.set_use_underline(true);
    let item_save = gtk::MenuItem::with_label("_Save As…");
    item_save.set_use_underline(true);
    let item_quit = gtk::MenuItem::with_label("_Quit");
    item_quit.set_use_underline(true);

    file_menu.append(&item_open);
    file_menu.append(&item_add);
    file_menu.append(&item_save);
    file_menu.append(&gtk::SeparatorMenuItem::new());
    file_menu.append(&item_quit);

    // ── Edit menu ───────────────────────────────────────────────────────────
    let edit_menu = gtk::Menu::new();
    let item_edit = gtk::MenuItem::with_label("_Edit");
    item_edit.set_use_underline(true);
    item_edit.set_submenu(Some(&edit_menu));

    let item_rot_ccw = gtk::MenuItem::with_label("Rotate ↺ 90° CCW");
    let item_rot_cw = gtk::MenuItem::with_label("Rotate ↻ 90° CW");
    let item_delete = gtk::MenuItem::with_label("_Delete Pages");
    item_delete.set_use_underline(true);
    let item_move_up = gtk::MenuItem::with_label("Move _Up");
    item_move_up.set_use_underline(true);
    let item_move_down = gtk::MenuItem::with_label("Move _Down");
    item_move_down.set_use_underline(true);

    edit_menu.append(&item_rot_ccw);
    edit_menu.append(&item_rot_cw);
    edit_menu.append(&gtk::SeparatorMenuItem::new());
    edit_menu.append(&item_delete);
    edit_menu.append(&gtk::SeparatorMenuItem::new());
    edit_menu.append(&item_move_up);
    edit_menu.append(&item_move_down);

    // ── Help menu ───────────────────────────────────────────────────────────
    let help_menu = gtk::Menu::new();
    let item_help = gtk::MenuItem::with_label("_Help");
    item_help.set_use_underline(true);
    item_help.set_submenu(Some(&help_menu));

    let item_about = gtk::MenuItem::with_label("_About");
    item_about.set_use_underline(true);
    help_menu.append(&item_about);

    menubar.append(&item_file);
    menubar.append(&item_edit);
    menubar.append(&item_help);

    // ── Signal handlers ──────────────────────────────────────────────────────

    item_open.connect_activate(clone!(@strong window, @strong state, @strong tree_view => move |_| {
        open_files_dialog(&window, &state, &tree_view, true);
    }));
    item_add.connect_activate(clone!(@strong window, @strong state, @strong tree_view => move |_| {
        open_files_dialog(&window, &state, &tree_view, false);
    }));
    item_save.connect_activate(clone!(@strong window, @strong state => move |_| {
        save_file_dialog(&window, &state);
    }));
    item_quit.connect_activate(clone!(@strong window => move |_| {
        window.close();
    }));

    item_rot_ccw.connect_activate(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().rotate_selected(&tree_view, 270);
    }));
    item_rot_cw.connect_activate(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().rotate_selected(&tree_view, 90);
    }));
    item_delete.connect_activate(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().delete_selected(&tree_view);
    }));
    item_move_up.connect_activate(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().move_up(&tree_view);
    }));
    item_move_down.connect_activate(clone!(@strong state, @strong tree_view => move |_| {
        state.borrow_mut().move_down(&tree_view);
    }));

    item_about.connect_activate(clone!(@strong window => move |_| {
        show_about_dialog(&window);
    }));

    menubar
}

// ─── File dialogs ─────────────────────────────────────────────────────────────

/// Open a file-chooser dialog for PDF files.
/// If `replace` is true the existing list is cleared first.
fn open_files_dialog(
    window: &ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    tree_view: &TreeView,
    replace: bool,
) {
    let dialog = FileChooserDialog::new(
        Some(if replace { "Open PDF Files" } else { "Add PDF Files" }),
        Some(window),
        FileChooserAction::Open,
    );
    dialog.set_select_multiple(true);

    let filter = FileFilter::new();
    filter.set_name(Some("PDF files"));
    filter.add_mime_type("application/pdf");
    filter.add_pattern("*.pdf");
    dialog.add_filter(filter);

    dialog.add_button("_Cancel", ResponseType::Cancel);
    dialog.add_button(if replace { "_Open" } else { "_Add" }, ResponseType::Accept);

    let response = dialog.run();
    let paths: Vec<PathBuf> = if response == ResponseType::Accept {
        dialog.filenames()
    } else {
        vec![]
    };
    dialog.close();

    if paths.is_empty() {
        return;
    }

    let mut st = state.borrow_mut();
    if replace {
        st.pages.clear();
    }
    let mut errors = Vec::new();
    for path in &paths {
        if let Err(e) = st.add_file(path.clone()) {
            errors.push(format!("{}: {e}", path.display()));
        }
    }
    st.refresh_store();
    drop(st);

    if !errors.is_empty() {
        let msg = errors.join("\n");
        let dlg = MessageDialog::new(
            Some(window),
            gtk::DialogFlags::MODAL,
            MessageType::Warning,
            gtk::ButtonsType::Ok,
            &format!("Could not load some files:\n{msg}"),
        );
        dlg.run();
        dlg.close();
    }

    // Clear sort so the user-defined order is preserved.
    let model = tree_view
        .model()
        .and_then(|m| m.downcast::<ListStore>().ok());
    if let Some(store) = model {
        store.set_sort_column_id(SortColumn::Index(COL_INDEX), SortType::Ascending);
    }
}

/// Open a save-file dialog and export the PDF.
fn save_file_dialog(window: &ApplicationWindow, state: &Rc<RefCell<AppState>>) {
    let dialog = FileChooserDialog::new(
        Some("Save PDF"),
        Some(window),
        FileChooserAction::Save,
    );
    dialog.set_do_overwrite_confirmation(true);
    dialog.set_current_name("output.pdf");

    let filter = FileFilter::new();
    filter.set_name(Some("PDF files"));
    filter.add_mime_type("application/pdf");
    filter.add_pattern("*.pdf");
    dialog.add_filter(filter);

    dialog.add_button("_Cancel", ResponseType::Cancel);
    dialog.add_button("_Save", ResponseType::Accept);

    let response = dialog.run();
    let path: Option<PathBuf> = if response == ResponseType::Accept {
        dialog.filename()
    } else {
        None
    };
    dialog.close();

    if let Some(path) = path {
        let mut st = state.borrow_mut();
        match st.export(&path) {
            Ok(()) => {
                st.statusbar.pop(st.status_ctx);
                st.statusbar.push(
                    st.status_ctx,
                    &format!("Saved to '{}'", path.display()),
                );
            }
            Err(e) => {
                drop(st);
                let dlg = MessageDialog::new(
                    Some(window),
                    gtk::DialogFlags::MODAL,
                    MessageType::Error,
                    gtk::ButtonsType::Ok,
                    &format!("Failed to save:\n{e}"),
                );
                dlg.run();
                dlg.close();
            }
        }
    }
}

// ─── About dialog ─────────────────────────────────────────────────────────────

fn show_about_dialog(window: &ApplicationWindow) {
    let dlg = AboutDialog::new();
    dlg.set_transient_for(Some(window));
    dlg.set_program_name("PDF Arranger");
    dlg.set_version(Some(env!("CARGO_PKG_VERSION")));
    dlg.set_comments(Some(
        "Merge, split, rotate, crop and rearrange PDF pages.",
    ));
    dlg.set_website(Some("https://github.com/pdfarranger/pdfarranger"));
    dlg.set_license_type(gtk::License::Gpl30);
    dlg.run();
    dlg.close();
}
