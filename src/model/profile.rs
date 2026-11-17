//! Profiles and their pages, and what covers each cell.

use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// 16 lowercase hex characters, random; given by `Config::repair`.
    pub id: String,
    /// The pad it belongs to. `pad` below is always that pad's kind.
    pub pad_id: String,
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    pub pages: Vec<Page>,
    pub active_page: usize,
    pub pad: PadKind,
}

impl Default for Profile {
    fn default() -> Self {
        let mut p = Profile {
            id: String::new(),
            pad_id: String::new(),
            name: "Profile".into(),
            rows: 3,
            cols: 5,
            pages: vec![Page::default()],
            active_page: 0,
            pad: PadKind::Main,
        };
        p.repair();
        p
    }
}

/// A cell covered by a stretched camera key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cover {
    pub owner: usize,
    /// This cell's place in the picture, column then row.
    pub at: [u8; 2],
    /// How many keys the picture covers, across then down.
    pub span: [u8; 2],
}

impl Profile {
    /// Takes back every "Always" for Home Assistant, keeping "Never": a
    /// profile that comes from a file was set up somewhere else, and whether
    /// Home Assistant may press a key on this PC is only for the person here
    /// to say.
    pub fn forget_remote_allows(&mut self) {
        for key in self.pages.iter_mut().flat_map(|page| page.keys.iter_mut()) {
            if key.remote == RemoteAccess::Allow {
                key.remote = RemoteAccess::Auto;
            }
        }
    }

    pub fn key_count(&self) -> usize {
        self.rows * self.cols
    }

    /// The keys of the page being shown.
    pub fn active_keys(&self) -> &[DockKey] {
        &self.pages[self.active_page].keys
    }

    /// The keys of the page being shown, to change.
    pub fn active_keys_mut(&mut self) -> &mut Vec<DockKey> {
        let page = self.active_page;
        &mut self.pages[page].keys
    }

    /// The rows and columns of keys with screens.
    pub(super) fn screen_grid(&self) -> (usize, usize) {
        match self.pad {
            PadKind::Main | PadKind::Dashboard => (self.rows, self.cols),
            PadKind::Akp03e => (SMALL_SCREEN_KEYS / 3, 3),
        }
    }

    /// The most keys a camera in this cell can cover, across then down.
    pub fn largest_span(&self, cell: usize) -> [u8; 2] {
        let (rows, cols) = self.screen_grid();
        let (row, column) = (cell / self.cols, cell % self.cols);
        if row >= rows || column >= cols {
            return [1, 1];
        }
        [(cols - column) as u8, (rows - row) as u8]
    }

    /// The stretched camera over a cell, if there is one. The keys under it
    /// stay as they were and come back when it's made smaller.
    pub fn cover(&self, page: usize, cell: usize) -> Option<Cover> {
        let keys = &self.pages.get(page)?.keys;
        let (cell_row, cell_column) = (cell / self.cols, cell % self.cols);
        let (rows, cols) = self.screen_grid();
        for row in 0..=cell_row.min(rows.saturating_sub(1)) {
            for column in 0..=cell_column.min(cols.saturating_sub(1)) {
                let owner = row * self.cols + column;
                let Some(key) = keys.get(owner).filter(|key| key.camera().is_some()) else { continue };
                let most = self.largest_span(owner);
                let span = [key.span[0].clamp(1, most[0]), key.span[1].clamp(1, most[1])];
                let inside = cell_column < column + span[0] as usize && cell_row < row + span[1] as usize;
                if span != [1, 1] && inside {
                    return Some(Cover { owner, at: [(cell_column - column) as u8, (cell_row - row) as u8], span });
                }
            }
        }
        None
    }

    /// The next empty key with a screen after this cell, going round the page.
    pub fn free_cell(&self, page: usize, from: usize) -> Option<usize> {
        let count = self.key_count().min(self.pages.get(page)?.keys.len());
        (from + 1..count).chain(0..from.min(count)).find(|cell| self.is_free(page, *cell))
    }

    /// The first empty key that can show something, for adding a key.
    pub fn first_free_cell(&self, page: usize) -> Option<usize> {
        let count = self.key_count().min(self.pages.get(page)?.keys.len());
        (0..count).find(|cell| self.is_free(page, *cell))
    }

    /// Empty, not under a stretched camera, and with a screen.
    pub(super) fn is_free(&self, page: usize, cell: usize) -> bool {
        let has_screen = self.pad.cell_has_screen(cell);
        let empty = self.pages[page].keys.get(cell).is_some_and(DockKey::is_empty);
        has_screen && empty && self.cover(page, cell).is_none()
    }

    /// Where a block of keys would land if dropped on this cell, kept on the pad.
    pub fn block_home(&self, cell: usize, span: [u8; 2]) -> usize {
        let (rows, cols) = self.screen_grid();
        let row = (cell / self.cols).min(rows.saturating_sub(span[1] as usize));
        let column = (cell % self.cols).min(cols.saturating_sub(span[0] as usize));
        row * self.cols + column
    }

    /// The cells a block of keys covers, in reading order.
    pub fn block_cells(&self, first: usize, span: [u8; 2]) -> Vec<usize> {
        let (across, down) = (span[0] as usize, span[1] as usize);
        (0..down).flat_map(|row| (0..across).map(move |column| first + row * self.cols + column)).collect()
    }

    /// What moves where when a stretched camera goes to a new place. Everything
    /// under it goes with it, and the keys it lands on take the space it leaves.
    /// Nothing happens if it would land on another stretched camera.
    pub fn block_moves(&self, page: usize, owner: usize, span: [u8; 2], to: usize) -> Option<Vec<(usize, usize)>> {
        let (rows, cols) = self.screen_grid();
        let fits = to / self.cols + span[1] as usize <= rows && to % self.cols + span[0] as usize <= cols;
        if !fits || to == owner {
            return None;
        }
        let (source, target) = (self.block_cells(owner, span), self.block_cells(to, span));
        if target.iter().any(|cell| self.cover(page, *cell).is_some_and(|cover| cover.owner != owner)) {
            return None;
        }
        let displaced = target.iter().copied().filter(|cell| !source.contains(cell));
        let freed = source.iter().copied().filter(|cell| !target.contains(cell));
        let mut moves: Vec<(usize, usize)> = displaced.zip(freed).collect();
        moves.extend(source.iter().copied().zip(target.iter().copied()));
        Some(moves)
    }

    /// The cell whose key really answers for this one.
    pub fn owner_of(&self, page: usize, cell: usize) -> usize {
        self.cover(page, cell).map_or(cell, |cover| cover.owner)
    }

    /// A page for an AKP03E. Its buttons and knobs start with the original
    /// app's rules for them.
    pub(super) fn small_pad_page() -> Page {
        let mut keys = vec![DockKey::default(); SMALL_ROWS * SMALL_COLS];
        let control = |title: &str, glyph: &str, color: [u8; 3], action: Option<Action>| DockKey {
            title: title.into(),
            glyph: glyph.into(),
            color,
            actions: action.into_iter().collect(),
            ..DockKey::default()
        };
        let gray = [70, 80, 100];

        keys[12] = control("Previous profile", "mdi:arrow-left", gray, Some(Action::PreviousProfile));
        keys[13] = control("First profile", "mdi:home", gray, Some(Action::FirstProfile));
        keys[14] = control("Next profile", "mdi:arrow-right", gray, Some(Action::NextProfile));

        let mut page = Page { keys };
        page.set_knob(0, KnobUse::Pages);
        page.set_knob(1, KnobUse::Volume);
        page.set_knob(2, KnobUse::Brightness);
        // The lower right knob's spare press mutes the microphone, which the
        // original's own profiles also put on that knob.
        let [.., press] = small_knob_cells(2);
        page.keys[press] = KnobUse::Microphone.keys(2)[2].clone();
        page
    }

    /// An empty profile for a virtual pad, with its own grid.
    pub fn for_dashboard(name: String, rows: usize, cols: usize) -> Profile {
        let mut profile = Profile { name, rows, cols, pages: vec![Page::default()], pad: PadKind::Dashboard, ..Profile::default() };
        profile.repair();
        profile
    }

    /// An empty profile for a pad of this kind. A virtual pad's takes the
    /// grid it's given.
    pub fn for_kind(kind: PadKind, name: String, grid: (usize, usize)) -> Profile {
        match kind {
            PadKind::Main => Profile { name, ..Profile::default() },
            PadKind::Akp03e => Profile::for_small_pad(name),
            PadKind::Dashboard => Profile::for_dashboard(name, grid.0, grid.1),
        }
    }

    pub fn for_small_pad(name: String) -> Profile {
        let mut profile = Profile {
            id: String::new(),
            pad_id: String::new(),
            name,
            rows: SMALL_ROWS,
            cols: SMALL_COLS,
            pages: vec![Profile::small_pad_page()],
            active_page: 0,
            pad: PadKind::Akp03e,
        };
        profile.repair();
        profile
    }

    /// The look of this profile's keys on its own pad.
    pub fn style_on_pad(&self, style: &KeyStyle) -> KeyStyle {
        let mut style = style.clone();
        if self.pad == PadKind::Akp03e {
            style.size = (style.size as u32 * 85 / SMALL_PICTURE).min(60) as u8;
        }
        style
    }

    /// Makes the data safe to index. Keys beyond the grid are kept, so making
    /// the grid smaller and then bigger again doesn't lose anything.
    pub fn repair(&mut self) {
        if self.pad == PadKind::Akp03e {
            self.rows = SMALL_ROWS;
            self.cols = SMALL_COLS;
        }
        self.rows = self.rows.clamp(1, 8);
        self.cols = self.cols.clamp(1, 10);
        if self.pages.is_empty() {
            self.pages.push(Page::default());
        }
        let n = self.key_count();
        for page in &mut self.pages {
            if page.keys.len() < n {
                page.keys.resize_with(n, DockKey::default);
            }
        }
        self.active_page = self.active_page.min(self.pages.len() - 1);
    }

    pub fn add_page(&mut self) {
        self.pages.push(match self.pad {
            PadKind::Main | PadKind::Dashboard => Page::default(),
            PadKind::Akp03e => Profile::small_pad_page(),
        });
        self.active_page = self.pages.len() - 1;
        self.repair();
    }
}
