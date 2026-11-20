//! A page as it is rendered: its canvas, the controls on it, and the
//! strips and lists laid over or under it as it scrolls.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
}

/// A rendered page in page coordinates. The status-bar band at its top is
/// left empty, and the `<`, the titles and the bar are laid over the page
/// as it scrolls.
pub struct Frame {
    pub canvas: Canvas,
    pub hits: Vec<Hit>,
    pub title: &'static str,
    pub back: bool,
    /// The page opens with its title in large type, scrolling away with it.
    pub header: bool,
    /// Where the page's content starts; once it slides under the top of
    /// the screen, the bar comes up over it.
    pub content_top: f32,
    /// The page sits on the About page's moving field.
    pub hero: Option<HeroArt>,
    /// Centre and span of the pattern grid, on the unlock page.
    pub pattern: Option<(f32, f32, f32)>,
    /// The colour of the page itself, past the canvas and under the bar.
    pub bg: Rgba,
    /// A list too long to draw whole: its rows are drawn as they scroll
    /// into view, below the canvas's content and past its end.
    pub rows: Option<Rows>,
    /// A strip that scrolls with the page until it reaches the bar, then
    /// stays under it.
    pub pinned: Option<Pinned>,
    /// A search bar under the large title that folds away as the page
    /// scrolls up.
    pub fold: Option<Fold>,
    /// A terminal's screen, or the editor's card, and the height of its
    /// rows: a drag on it scrolls through what it holds, rather than moving
    /// the page.
    pub scrollback: Option<(Rect, f32)>,
    /// A strip held at the bottom of the screen over the page; the page
    /// scrolls far enough for nothing to stay under it.
    pub dock: Option<Dock>,
    /// A text field's blinking cursor, in page coordinates.
    pub caret: Option<widgets::Caret>,
    /// The round button at the right end of the bar: what it shows, and
    /// what touching it does.
    pub button: Option<(widgets::Glyph, Action)>,
}

/// A strip held at the bottom of the screen over the page, such as the
/// file manager's choice of where a picked entry goes.
pub struct Dock {
    pub canvas: Canvas,
    /// In the strip's own coordinates.
    pub hits: Vec<Hit>,
}

/// The rows of a list drawn as they come into view, the way a list view
/// binds its rows: where each lies on the page and what touching one does,
/// for rows that answer touch.
pub struct Rows {
    /// Page y of each row's top, then of the last row's bottom; whole
    /// pixels, so the rows meet without seams.
    pub edges: Vec<f32>,
    pub action: Option<fn(usize) -> Action>,
}

impl Rows {
    pub fn count(&self) -> usize {
        self.edges.len() - 1
    }

    pub fn bottom(&self) -> f32 {
        *self.edges.last().expect("a list has its top edge at least")
    }

    /// The row under page height `y`.
    pub fn at(&self, y: f32) -> Option<usize> {
        if y < self.edges[0] || y >= self.bottom() {
            return None;
        }
        Some(self.edges.partition_point(|&e| e <= y) - 1)
    }

    /// The rows reaching into the band from `y0` to `y1`.
    pub fn within(&self, y0: f32, y1: f32) -> std::ops::Range<usize> {
        let first = self.edges.partition_point(|&e| e <= y0).saturating_sub(1);
        let end = self.edges.partition_point(|&e| e < y1).min(self.count());
        first.min(end)..end
    }

    /// Where row `i` lies on the page.
    pub fn rect(&self, i: usize, w: f32) -> Rect {
        Rect::new(0.0, self.edges[i], w, self.edges[i + 1] - self.edges[i])
    }
}

/// A strip drawn once and laid over the page at `top`, or just under the
/// bar once the page has scrolled it that far; its controls answer where
/// it shows.
pub struct Pinned {
    pub canvas: Canvas,
    pub top: f32,
    /// In the strip's own coordinates.
    pub hits: Vec<Hit>,
}

/// A search bar under a page's large title folding away as the page scrolls
/// up, as the stock settings home's does: its top stays put and its foot
/// rises with the page, its icon and words fading out first and then the
/// bar, until nothing of it is left. The content under it then closes the
/// gap it leaves, and only after that does the large title go up with the
/// page. A page let go with its bar part folded settles folded or unfolded,
/// whichever is nearer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fold {
    /// Where the bar stands unfolded, in page coordinates.
    pub bar: Rect,
    /// What the bar asks for, in grey.
    pub hint: &'static str,
    /// How far the page scrolls with its large title held still: the bar
    /// and the gap under it.
    pub span: f32,
}

impl Fold {
    /// What shows of the bar, on screen, with the page `offset` pixels
    /// down: all of it, pulled down with the page past its top, or as much
    /// as is left of it folding; none once it has folded away.
    pub fn shown(&self, offset: f32) -> Option<Rect> {
        let h = self.bar.h - offset.max(0.0);
        (h > 0.0).then(|| Rect { y: self.bar.y - offset.min(0.0), h, ..self.bar })
    }

    /// How strongly the bar, and its icon and words, show with the page
    /// `offset` pixels down: the words fade out over the first stretch of
    /// the fold, and the bar over the next.
    pub fn levels(&self, t: &Theme, offset: f32) -> (f32, f32) {
        let along = offset.max(0.0) / t.m.dp(FOLD_FADE_DP);
        ((2.0 - along).clamp(0.0, 1.0), (1.0 - along).clamp(0.0, 1.0))
    }

    /// Draw what shows of the bar with the page `offset` pixels down, its
    /// icon and words laid from `words`, which [`widgets::search_words`]
    /// makes.
    pub fn draw(&self, out: &mut Canvas, t: &Theme, words: &Canvas, offset: f32) {
        let Some(r) = self.shown(offset) else { return };
        let (bar, level) = self.levels(t, offset);
        widgets::search_bar(out, t, r, bar);
        // The icon and words stay centred in what is left of the bar.
        let sink = r.y + r.h / 2.0 - (self.bar.y + self.bar.h / 2.0);
        out.lay(words, self.bar.x.floor() as i32, (self.bar.y.floor() + sink).round() as i32, level);
    }

    /// How far the large title has gone up with the page `offset` pixels
    /// down: nowhere while the bar folds and its gap closes, then as far
    /// as the page has gone since; down with a page pulled past its top.
    pub fn lift(&self, offset: f32) -> f32 {
        if offset < 0.0 {
            offset
        } else {
            (offset - self.span).max(0.0)
        }
    }

    /// Where a page let go at `scroll` comes to rest: with the bar part
    /// folded, folded or unfolded, whichever is nearer; anywhere else,
    /// where it is.
    pub fn settle(&self, scroll: f32) -> f32 {
        match scroll {
            s if s <= 0.0 || s >= self.span => s,
            s if s < self.span * FOLD_SETTLE => 0.0,
            _ => self.span,
        }
    }
}

/// What the About page lays over its field: the wordmark, the version and
/// who made the recovery, which stay put while the cards scroll, and the
/// cards' coverage row by row.
pub struct HeroArt {
    pub wordmark: Sprite,
    pub version: Sprite,
    pub author: Sprite,
    pub runs: Vec<Vec<Run>>,
}

impl Frame {
    /// A page of cards on the grey page colour that opens with its title in
    /// large type and goes back with `<`.
    pub fn new(t: &Theme, canvas: Canvas, hits: Vec<Hit>, title: &'static str, content_top: f32) -> Frame {
        Frame {
            canvas,
            hits,
            title,
            back: true,
            header: true,
            content_top,
            hero: None,
            pattern: None,
            bg: t.pal.bg,
            rows: None,
            pinned: None,
            fold: None,
            scrollback: None,
            dock: None,
            caret: None,
            button: None,
        }
    }

    /// Where the control at `r` on the page shows, on screen, with the page
    /// `offset` pixels down: the folding search bar as much as is left of
    /// it, and none once folded away; any other with the page.
    pub fn shown(&self, r: Rect, offset: f32) -> Option<Rect> {
        match &self.fold {
            Some(fold) if fold.bar == r => fold.shown(offset),
            _ => Some(Rect { y: r.y - offset, ..r }),
        }
    }

    /// How far the large title has gone up with the page `offset` pixels
    /// down: as far as the page, but for a search bar folding under it.
    pub fn title_lift(&self, offset: f32) -> f32 {
        self.fold.as_ref().map_or(offset, |f| f.lift(offset))
    }

    /// Where a page let go at `scroll` comes to rest: see [`Fold::settle`].
    pub fn settle(&self, scroll: f32) -> f32 {
        self.fold.as_ref().map_or(scroll, |f| f.settle(scroll))
    }

    /// The topmost control under a point in page coordinates.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if let Some(h) = self.hits.iter().rev().find(|h| h.rect.contains(x, y)) {
            return Some(*h);
        }
        let rows = self.rows.as_ref()?;
        let action = rows.action?;
        let i = rows.at(y)?;
        Some(Hit { rect: rows.rect(i, self.canvas.w as f32), action: action(i) })
    }

    /// How tall the page is: its canvas, or its rows and the margin after
    /// them if they reach further.
    pub fn height(&self, t: &Theme) -> f32 {
        let h = self.canvas.h as f32;
        let h = self.rows.as_ref().map_or(h, |r| h.max((r.bottom() + t.m.margin * 2.0).ceil()));
        h + self.dock.as_ref().map_or(0.0, |d| d.canvas.h as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_bar_pulled_down_past_the_top_comes_down_whole_with_its_title() {
        let fold = Fold { bar: Rect::new(36.0, 503.0, 1128.0, 131.0), hint: "", span: 169.0 };
        assert_eq!(fold.shown(-30.0), Some(Rect::new(36.0, 533.0, 1128.0, 131.0)));
        assert_eq!(fold.lift(-30.0), -30.0);
        assert_eq!(fold.shown(31.0), Some(Rect::new(36.0, 503.0, 1128.0, 100.0)));
        assert_eq!(fold.shown(131.0), None);
        assert_eq!((fold.lift(150.0), fold.lift(200.0)), (0.0, 31.0));
    }

    #[test]
    fn rows_are_found_by_where_they_lie() {
        let rows = Rows { edges: vec![100.0, 352.0, 682.0, 934.0], action: Some(Action::OpenEntry) };
        assert_eq!(rows.count(), 3);
        assert_eq!(rows.at(99.0), None);
        assert_eq!(rows.at(100.0), Some(0));
        assert_eq!(rows.at(352.0), Some(1));
        assert_eq!(rows.at(933.0), Some(2));
        assert_eq!(rows.at(934.0), None);
        assert_eq!(rows.within(0.0, 100.0), 0..0);
        assert_eq!(rows.within(0.0, 101.0), 0..1);
        assert_eq!(rows.within(351.0, 700.0), 0..3);
        assert_eq!(rows.within(700.0, 5000.0), 2..3);
        assert_eq!(rows.within(934.0, 5000.0), 3..3);
    }
}
