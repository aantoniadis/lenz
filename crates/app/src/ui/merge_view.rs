//! The Merge View: Ours | Result | Theirs, the Base on a toggle, block-level resolution with undo, saved through
//! the safe writer. Laid out as two 2-way views sharing the Result pane: Ours ⟷ Result and Result ⟷ Theirs, each
//! built from the merge blocks, kept at the same Result line.

use std::sync::atomic::{AtomicU8, Ordering};

use super::*;
use lenz_content::find::FindOption;
use crate::merge::{BlockState, Pick};

/// The process exit code under `git mergetool`: 0 only after Done, so closing or cancelling makes git restore
/// its backup.
static EXIT: AtomicU8 = AtomicU8::new(0);

pub fn exit_code() -> u8 {
    EXIT.load(Ordering::Relaxed)
}

pub(super) fn set_exit(code: u8) {
    EXIT.store(code, Ordering::Relaxed);
}

pub use crate::merge_session::{Act, Ask, Control, Opened, open_files};
use crate::merge_session::{MergeSession, owner};

pub(super) struct MergeState {
    core: MergeSession,
    pub controls: Vec<(f32, f32, Control)>,
    pub row_blocks: [Vec<(f32, usize)>; 3],
    pub pane_w: [f32;3],
    pub horizontal: [f32;3],
    columns: [Option<usize>;3],
    horizontal_version: (u64,bool),
}
impl std::ops::Deref for MergeState {
    type Target = MergeSession;
    fn deref(&self) -> &MergeSession {
        &self.core
    }
}
impl std::ops::DerefMut for MergeState {
    fn deref_mut(&mut self) -> &mut MergeSession {
        &mut self.core
    }
}
impl MergeState {
    pub(super) fn pane_at(&self,x:f32)->usize { if x<self.pane_w[0]+GUTTER_W {0} else if x<self.pane_w[0]+self.pane_w[1]+2.0*GUTTER_W {1} else {2} }
    pub fn new(o: Opened, rows: f32) -> Self {
        Self {
            core: MergeSession::new(o, rows),
            controls: Vec::new(),
            row_blocks: Default::default(),
            pane_w: [400.0;3],
            horizontal:[0.0;3],columns:[None;3],horizontal_version:(0,false),
        }
    }
    /// The merge block on the row under `(x, y)` (pixels from the panes' top-left).
    fn block_at(&self, x: f32, y: f32) -> Option<usize> {
        let p = self.pane_at(x);
        let row = y / line_h();
        self.row_blocks[p]
            .iter()
            .find(|(ry, _)| (*ry..ry + 1.0).contains(&row))
            .map(|r| r.1)
    }
}

/// Everything the canvas paints for a Merge View.
pub(super) struct MergeFrame {
    widths: [f32;3],
    panes: [Vec<Painted>; 3],
    horizontal:[f32;3],
    connectors: [Vec<Connector>; 2],
    controls: Vec<(f32, f32, &'static str, bool)>,
    headers: [String; 3],
    /// The banner: a question, or (`true`) a failed save in red.
    ask: Option<(String, bool)>,
    current: Option<(f32, f32)>,
    /// Middle-pane row spans of Conflicts the magic wand resolved and nobody has looked at yet.
    auto: Vec<(f32, f32)>,
    /// Result rows of blocks resumed from the file ("edited outside Lenz").
    outside: Vec<(f32, f32)>,
}

impl Viewer {
    /// Show `opened` instead of the 2-way panes. `mergetool`: the window is one `git mergetool` file.
    pub fn start_merge(&mut self, opened: Opened, mergetool: bool, cx: &mut Context<Self>) {
        if !self.edits_may_close(cx) {return;}
        if mergetool {
            EXIT.store(1, Ordering::Relaxed);
            self.layout.set_files(false);
        }
        let mut s = MergeState::new(opened, self.comparison.view.viewport_rows());
        s.mergetool = mergetool;
        self.merge = Some(s);
        cx.notify();
    }

    pub(super) fn merge_title(&self) -> Option<String> {
        let s = self.merge.as_ref()?;
        Some(format!(
            "{} ⟷ {} — {}{} — Lenz",
            s.labels.ours,
            s.labels.theirs,
            s.path,
            if s.dirty { " ●" } else { "" }
        ))
    }

    /// Lay out the Merge View for this frame (and remember the gutter controls and rows for clicks).
    pub(super) fn merge_frame(&mut self, rows: f32, pane_w: [f32;3]) -> Option<MergeFrame> {
        let s = self.merge.as_mut()?;
        s.pane_w=pane_w;
        for p in 0..3{s.scroll_horizontal(p,0.0,self.char_w);}
        s.set_viewport_rows(rows);
        let (mut panes, mut connectors) = s.layout();
        let m = &s.m;
        let n = m.blocks().len();
        let mid = |i: usize| {
            if s.base_shown {
                m.blocks()[i].base.clone()
            } else {
                m.result_range(i)
            }
        };
        // tints by merge state rather than by the 2-way diffs the layout was built from
        let mut row_blocks: [Vec<(f32, usize)>; 3] = Default::default();
        for (p, rows) in panes.iter_mut().enumerate() {
            for r in rows.iter_mut() {
                if let Content::Line(l) = r.content {
                    let owner = owner(n, l, |i| match p {
                        0 => m.blocks()[i].left.clone(),
                        1 => mid(i),
                        _ => m.blocks()[i].right.clone(),
                    });
                    r.tint = owner.and_then(|i| s.tint(p, i));
                    if r.tint.is_none() {
                        r.inline.clear();
                    }
                    if let Some(i) = owner {
                        row_blocks[p].push((r.y, i));
                    }
                }
            }
        }
        for (g, cs) in connectors.iter_mut().enumerate() {
            cs.retain_mut(|c| match s.tint(if g == 0 { 0 } else { 2 }, c.block) {
                Some((kind, ignored)) => {
                    c.kind = kind;
                    c.ignored = ignored;
                    true
                }
                None => false,
            });
        }
        // gutter controls at each block's top on the version's side; none while the Base is shown
        let mut controls = Vec::new();
        let mut placed = Vec::new();
        for (g, cs) in connectors.iter().enumerate().filter(|_| !s.base_shown) {
            let pick = if g == 0 { Pick::Ours } else { Pick::Theirs };
            let gx = if g == 0 {
                pane_w[0]
            } else {
                pane_w[0] + pane_w[1] + GUTTER_W
            };
            let at = |k: f32| {
                if g == 0 {
                    gx + 2.0 + k * 18.0
                } else {
                    gx + GUTTER_W - 18.0 - k * 18.0
                }
            };
            for c in cs {
                let i = c.block;
                let b = &m.blocks()[i];
                let ch = m.choice(i);
                let y = (if g == 0 { c.left.0 } else { c.right.0 }) * line_h();
                let changed = if g == 0 {
                    b.left_changed
                } else {
                    b.right_changed
                };
                let mut k = 0.0;
                if changed && !(ch.resolved && ch.custom.is_none() && ch.taken.contains(&pick)) {
                    controls.push((at(k), y, if g == 0 { "≫" } else { "≪" }, false));
                    placed.push((at(k), y, Control::Accept(i, pick)));
                    k += 1.0;
                }
                let applied_here = ch.taken.contains(&pick) || (g == 0 && ch.custom.is_some());
                if m.state(i) == BlockState::Unresolved && g == 0
                    || (!m.is_conflict(i) && applied_here)
                {
                    controls.push((at(k), y, "✕", false));
                    placed.push((at(k), y, Control::KeepBase(i)));
                    k += 1.0;
                }
                if g == 0 && m.is_conflict(i) && m.state(i) == BlockState::Resolved {
                    controls.push((at(k), y, "↺", ch.auto));
                    placed.push((at(k), y, Control::Reopen(i)));
                }
            }
        }
        let current = s.current.and_then(|i| {
            let r = mid(i);
            let ys: Vec<f32> = panes[1]
                .iter()
                .filter(|row| matches!(row.content, Content::Line(l) if r.contains(&l)))
                .map(|row| row.y)
                .collect();
            match (ys.first(), ys.last()) {
                (Some(&a), Some(&b)) => Some((a, b + 1.0)),
                _ => connectors[0].iter().find(|c| c.block == i).map(|c| c.right),
            }
        });
        let span = |i: usize| {
            let r = mid(i);
            let ys: Vec<f32> = panes[1]
                .iter()
                .filter(|row| matches!(row.content, Content::Line(l) if r.contains(&l)))
                .map(|row| row.y)
                .collect();
            Some((*ys.first()?, ys.last()? + 1.0))
        };
        let auto = (0..n)
            .filter(|&i| m.choice(i).auto && !s.base_shown)
            .filter_map(span)
            .collect();
        let outside = (0..n)
            .filter(|&i| m.choice(i).outside && !s.base_shown)
            .filter_map(span)
            .collect();
        let painted = [
            resolve(&self.palette,
                &panes[0],
                m.side(Some(Pick::Ours)),
                s.parsed_pane(0),
                None,
                s.find_hits(0),
            ),
            resolve(&self.palette,
                &panes[1],
                s.mid_lines(),
                s.parsed_pane(1),
                None,
                s.find_hits(1),
            ),
            resolve(&self.palette,
                &panes[2],
                m.side(Some(Pick::Theirs)),
                s.parsed_pane(2),
                None,
                s.find_hits(2),
            ),
        ];
        let ask = s.ask.map(|a| match a {
            Ask::DoneWithConflicts => format!(
                "{} — Done writes conflict markers{}. ctrl-enter again to finish, Esc to go back",
                s.counters(),
                if s.mergetool { " and git stages them" } else { "" }
            ),
            Ask::Close if s.dirty && !s.mergetool => "Unsaved changes — ctrl-s saves; Esc again discards them".to_string(),
            Ask::Close if s.dirty => "Unsaved changes — ctrl-s saves; Esc again closes without saving".to_string(),
            Ask::Close => "git will restore the original and discard what you saved — ctrl-enter (Done) keeps it; Esc again discards".to_string(),
            Ask::ChangedOnDisk => "Changed on disk — r reloads it (discards your choices) · Esc keeps mine".to_string(),
            Ask::Overwrite { done } => format!(
                "{} changed on disk since Lenz read it — {} again overwrites it · r reloads it (discards your choices) · Esc cancels",
                s.path,
                if done { "ctrl-enter" } else { "ctrl-s" }
            ),
        });
        let headers = s.headers();
        s.controls = placed;
        s.row_blocks = row_blocks;
        s.pane_w = pane_w;
        let ask = ask
            .map(|a| (a, false))
            .or_else(|| s.error.clone().map(|e| (e, true)))
            .or_else(|| s.find_notice().map(|n| (n, false)));
        Some(MergeFrame {
            widths:pane_w,
            horizontal:s.horizontal,
            panes: painted,
            connectors,
            controls,
            headers,
            ask,
            current,
            auto,
            outside,
        })
    }

    /// Parse the visible versions once and the Result after each change, in the background.
    pub(super) fn merge_reparse(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.merge.as_mut() else { return };
        for job in s.parse_jobs() {
            cx.spawn(async move |this, cx| {
                let (job, parsed) = cx
                    .background_executor()
                    .spawn(async move { job.run() })
                    .await;
                this.update(cx, |v: &mut Viewer, cx| {
                    if let Some(s) = v.merge.as_mut()
                        && s.finish_parse(&job, parsed)
                    {
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// The Merge View's notices for the corner, in place of the hidden 2-way Comparison's.
    pub(super) fn merge_notices(&self) -> Option<Vec<String>> {
        self.merge.as_ref().map(|s| s.notices())
    }

    /// Coming back to the window: has the merged file changed on disk? (The cheap size and mtime check.)
    pub fn check_merge_on_disk(&mut self, cx: &mut Context<Self>) {
        let merge_changed = self.merge.as_mut().is_some_and(|s| s.check_disk());
        let picker_changed = self.picker_v.as_mut().is_some_and(|s| s.check_disk());
        if merge_changed || picker_changed {
            self.menu = None;
            self.command_palette = None;
            self.diff_options = None;
            cx.notify();
        }
    }

    /// Mouse down in the diff area of a Merge View: a gutter control, a block (made current), or the Result's
    /// header (next unresolved Conflict). `x`, `y` relative to the top-left of the panes, below the header.
    pub(super) fn merge_click(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.merge.as_mut() else {
            return false;
        };
        if y < 0.0 {
            self.merge_act(Act::NextConflict, cx);
            return true;
        }
        let pane = s.pane_at(x);
        s.focus_pane(pane);
        let hit = s
            .controls
            .iter()
            .find(|(cx_, cy, _)| {
                (cx_ - 2.0..cx_ + 16.0).contains(&x) && (*cy..cy + line_h()).contains(&y)
            })
            .map(|c| c.2);
        if let Some(c) = hit {
            self.merge_control(c, cx);
        } else if let Some(i) = s.block_at(x, y) {
            s.focus_block(i);
            cx.notify();
        }
        true
    }

    pub(super) fn merge_control(&mut self, c: Control, cx: &mut Context<Self>) {
        if self.resize_blocked(){return;}
        if let Some(s) = self.merge.as_mut() {
            s.control(c);
            cx.notify();
        }
    }

    /// Right-click on a block: what can be done with it.
    pub(super) fn merge_menu(&mut self, x: f32, y: f32) -> Option<Vec<(String, MenuAction)>> {
        let s = self.merge.as_mut()?;
        let i = s.block_at(x, y)?;
        s.focus_block(i);
        let items = s
            .controls_for(i)
            .into_iter()
            .map(|c| (crate::catalog::pointer::control(c).to_string(), MenuAction::Merge(c)))
            .collect();
        Some(items)
    }

    pub(super) fn merge_act(&mut self, act: Act, cx: &mut Context<Self>) {
        if self.about.is_some()||self.theme_chooser.is_some()||self.discard_modal()
            ||self.help||self.popup.is_some()||self.goto.is_some()||self.switcher.is_some()||self.menu.is_some()
            ||self.search.as_ref().is_some_and(|s|s.editing)||self.conflicts.as_ref().is_some_and(|s|s.confirming()) {return;}
        if self.merge.as_ref().is_some_and(|s|s.ask.is_some()) && !matches!(act,Act::Save|Act::Done|Act::Cancel|Act::Reload){return;}
        let Some(s) = self.merge.as_mut() else { return };
        let outcome = s.execute(act);
        if let Some(code) = outcome.quit && self.edits_may_close(cx) {set_exit(code);cx.quit();}
        if let Some(t) = outcome.toast {
            self.toast(t, cx);
        }
        if outcome.done {
            self.conflict_done(cx);
        }
        if outcome.back {
            self.conflict_cancel(cx);
        }
        if outcome.mark {
            self.mark_current_resolved(cx);
        }
        if let Some(forward) = outcome.cross {
            self.cross_to_unresolved(forward, cx);
        }
        cx.notify();
    }

    /// Whether the window may close now (the title bar's close button); otherwise asks first.
    pub fn merge_may_close(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.merge.as_mut() else {
            return true;
        };
        let ready = s.may_close();
        if !ready {
            self.menu = None;
            self.command_palette = None;
            self.diff_options = None;
            cx.notify();
        }
        ready
    }

    /// Keys of the Merge View; false for keys it leaves to the viewer (scrolling, find, help…).
    pub(super) fn merge_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.merge.as_mut() else {
            return false;
        };
        let k = &ev.keystroke;
        let (key, shift, alt, ctrl) = (
            k.key.as_str(),
            k.modifiers.shift,
            k.modifiers.alt,
            k.modifiers.control || k.modifiers.platform,
        );
        if s.ask.is_some() && !matches!((key, shift, alt, ctrl),
            ("s" | "enter", false, false, true)
            | ("escape", ..)
            | ("r" | "q", false, false, false)
            | ("w" | "q", false, false, true)) {
            return true;
        }
        if ctrl && alt && !shift && matches!(key,"left"|"right") && s.ask.is_none() && !s.find_editing(){
            s.scroll_horizontal(s.find_pane(),if key=="left"{-4.0*self.char_w}else{4.0*self.char_w},self.char_w);cx.notify();return true;
        }
        if key==":" {self.open_palette(cx);return true;}
        let current = s
            .current
            .or_else(|| (0..s.m.blocks().len()).find(|&i| s.m.state(i) == BlockState::Unresolved));
        let mergetool = s.mergetool;
        if s.ask.is_none() && s.find_editing() && alt && !ctrl && let Some(option) = match key { "c" => Some(FindOption::Case), "w" => Some(FindOption::Word), "r" => Some(FindOption::Regex), _ => None } {
            self.comparison.find_options = s.toggle_find_option(option);
            cx.notify();
            return true;
        }
        if s.ask.is_none() && s.find_editing() {
            match (key, ctrl) {
                ("escape", _) => {
                    s.dismiss_find();
                }
                ("enter", _) => s.finish_find(),
                ("backspace", _) => s.backspace_find(),
                ("tab", _) => s.cycle_find_pane(),
                ("f3", _) => {
                    s.find_step(!shift);
                }
                ("a", true) => s.clear_find_query(),
                ("v", true) => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        s.type_find(&text);
                    }
                }
                _ => {
                    if let Some(text) = Viewer::typed(ev) {
                        s.type_find(&text);
                    } else {
                        return false;
                    }
                }
            }
            cx.notify();
            return true;
        }
        if (key == "/" && !ctrl && !alt) || (key == "f" && ctrl && !shift) {
            self.comparison.find = None;
            s.begin_find(self.comparison.find_options);
            cx.notify();
            return true;
        }
        if key == "f3" {
            s.find_step(!shift);
            cx.notify();
            return true;
        }
        if key == "escape" && s.ask.is_none() && s.dismiss_find() {
            cx.notify();
            return true;
        }
        if key == "tab" && !ctrl && !alt {
            s.cycle_find_pane();
            cx.notify();
            return true;
        }
        let mut on_current = |s: &mut MergeState, pick: u8| match current {
            Some(i) => {
                let c = match pick {
                    1 => Control::Accept(i, Pick::Ours),
                    2 => Control::Accept(i, Pick::Theirs),
                    3 => Control::Both(i),
                    4 => Control::KeepBase(i),
                    _ => unreachable!(),
                };
                if s.control(c) {
                    cx.notify();
                }
                true
            }
            None => false,
        };
        let act = match (key, shift, alt, ctrl) {
            ("right", true, true, false) => return on_current(s, 1) || self.merge_none(cx),
            ("left", true, true, false) => return on_current(s, 2) || self.merge_none(cx),
            ("down", true, true, false) => return on_current(s, 3) || self.merge_none(cx),
            ("backspace", true, true, false) => return on_current(s, 4) || self.merge_none(cx),
            ("n" | "f7", false, false, false) | ("down", false, true, false) => Act::NextChange,
            ("p", false, false, false)
            | ("f7", true, false, false)
            | ("up", false, true, false) => Act::PrevChange,
            ("n", true, false, false) => Act::NextConflict,
            ("p", true, false, false) => Act::PrevConflict,
            ("z", false, false, true) => Act::Undo,
            ("z", true, false, true) | ("y", false, false, true) => Act::Redo,
            ("r", true, false, true) => Act::StartOver,
            ("r", false, false, false)
                if matches!(s.ask, Some(Ask::ChangedOnDisk | Ask::Overwrite { .. })) =>
            {
                Act::Reload
            }
            ("s", false, false, true) => Act::Save,
            ("enter", false, false, true) => Act::Done,
            // backing out of a question (Keep mine, for the changed-on-disk banner) or dismissing a failed save
            ("escape", ..)
                if s.ask.is_some_and(|a| a != Ask::Close)
                    || (s.ask.is_none() && s.error.is_some()) =>
            {
                s.dismiss();
                cx.notify();
                return true;
            }
            ("escape", ..) => Act::Cancel,
            ("q", false, false, false) | ("w", false, false, true) | ("q", false, false, true)
                if mergetool =>
            {
                Act::Cancel
            }
            // the Repository Window's quit keys still quit, asking first about unsaved work
            ("q", false, false, false) | ("w", false, false, true) | ("q", false, false, true) => {
                if self.edits_may_close(cx) && self.merge_may_close(cx) && self.conflicts_may_close(cx) {
                    cx.quit();
                }
                return true;
            }
            ("b", false, false, false) => Act::ToggleBase,
            ("w", false, false, false) => Act::Whitespace,
            ("s", false, false, false) => Act::Scroll,
            ("x" | "i" | "z" | "h", false, false, false) => {
                self.toast("Not available in the Merge View".into(), cx);
                return true;
            }
            _ => return false,
        };
        self.merge_act(act, cx);
        true
    }

    fn merge_none(&mut self, cx: &mut Context<Self>) -> bool {
        self.toast("No change here".into(), cx);
        true
    }

}

/// Paint the Merge View into the diff area `(x0, top, w, h)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(palette: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
    font: &Font,
    x0: f32,
    top: f32,
    w: f32,
    h: f32,
    f: &MergeFrame,
) {
    let bounds = Bounds {
        origin: point(px(x0), px(top)),
        size: size(px(w), px(h)),
    };
    fill(window, x0, top, w, h, palette.bg);
    let widths=f.widths;
    let xs = [x0, x0 + widths[0] + GUTTER_W, x0 + widths[0]+widths[1]+2.0*GUTTER_W];
    fill(window, x0, top, w, hdr_h(), palette.header_bg);
    for (i, head) in f.headers.iter().enumerate() {
        window.with_content_mask(Some(clip(bounds, xs[i], widths[i])), |window| {
            text(window, cx, font, xs[i] + 8.0, top + 2.0, head, palette.fg)
        });
    }
    let y0 = top + hdr_h();
    let body = Bounds {
        origin: point(px(x0), px(y0)),
        size: size(px(w.max(0.0)), px((h - hdr_h()).max(0.0))),
    };
    for (i, rows) in f.panes.iter().enumerate() {
        paint_side(palette, window, cx, font, body, xs[i], widths[i], f.horizontal[i], 0.0, rows);
    }
    // wand-resolved, not yet looked at: a dotted edge down the Result's left side
    for &(a, b) in &f.auto {
        let mut y = y0 + a * line_h();
        while y < y0 + b * line_h() {
            fill(window, xs[1] + 5.0, y, 3.0, 4.0, palette.accent);
            y += 7.0;
        }
    }
    if let Some((a, b)) = f.current {
        fill(
            window,
            xs[1],
            y0 + a * line_h(),
            3.0,
            (b - a).max(0.1) * line_h(),
            palette.accent,
        );
    }
    // "edited outside Lenz": an edge on the Result's far side
    for &(a, b) in &f.outside {
        fill(
            window,
            xs[1] + widths[1] - 3.0,
            y0 + a * line_h(),
            3.0,
            (b - a).max(0.1) * line_h(),
            palette.status[3],
        );
    }
    for (i, (x, connectors)) in xs.iter().zip(&f.connectors).enumerate() {
        fill(window, x + widths[i], y0, GUTTER_W, h - hdr_h(), palette.gutter_bg);
        paint_bands(palette, window, body, *x, [widths[i],widths[i+1]], connectors);
    }
    window.with_content_mask(Some(ContentMask { bounds: body }), |window| {
        for &(x, y, glyph, auto) in &f.controls {
            let (cx_, cy) = (x0 + x, y0 + y);
            fill(
                window,
                cx_ - 1.0,
                cy + 1.0,
                16.0,
                line_h() - 2.0,
                if auto {
                    palette.banner_bg
                } else {
                    palette.status_bg
                },
            );
            text(window, cx, font, cx_ + 2.0, cy, glyph, palette.accent);
        }
    });
    if let Some((ask, failed)) = &f.ask {
        let bh = line_h() + 8.0;
        fill(
            window,
            x0,
            y0,
            w,
            bh,
            if *failed {
                tint(palette, Kind::Conflict)
            } else {
                palette.banner_bg
            },
        );
        if *failed {
            fill(window, x0, y0, 4.0, bh, edge(palette, Kind::Conflict));
        }
        text(window, cx, font, x0 + 12.0, y0 + 4.0, ask, palette.fg);
    }
}

/// Bezier bands across a gutter between a version and the Result, and markers where a side is empty
/// (the 2-way Connectors without fold chevrons: the gutter holds the merge controls instead).
fn paint_bands(palette: &theme::Palette,
    window: &mut Window,
    bounds: Bounds<Pixels>,
    x0: f32,
    widths: [f32;2],
    connectors: &[Connector],
) {
    let y0 = f32::from(bounds.origin.y);
    let pane_w=widths[0];
    let (gx, gx1) = (x0 + pane_w, x0 + pane_w + GUTTER_W);
    let (mid0, mid1) = (gx + GUTTER_W * 0.45, gx + GUTTER_W * 0.55);
    let y = |rows: f32| y0 + rows * line_h();
    window.with_content_mask(Some(clip(bounds, gx, GUTTER_W)), |window| {
        for c in connectors {
            let (lt, rt) = (y(c.left.0), y(c.right.0));
            let (lb, rb) = (y(c.left.1).max(lt + 1.0), y(c.right.1).max(rt + 1.0));
            let mut band = PathBuilder::fill();
            band.move_to(point(px(gx), px(lt)));
            band.cubic_bezier_to(
                point(px(gx1), px(rt)),
                point(px(mid0), px(lt)),
                point(px(mid1), px(rt)),
            );
            band.line_to(point(px(gx1), px(rb)));
            band.cubic_bezier_to(
                point(px(gx), px(lb)),
                point(px(mid1), px(rb)),
                point(px(mid0), px(lb)),
            );
            band.close();
            if let Ok(path) = band.build() {
                window.paint_path(path, Hsla::from(rgb(shade(palette, c.kind, c.ignored, tint))));
            }
        }
    });
    for c in connectors {
        let color = shade(palette, c.kind, c.ignored, edge);
        if c.left.0 == c.left.1 {
            fill(window, x0, y(c.left.0) - 1.0, pane_w, 2.0, color);
        }
        if c.right.0 == c.right.1 {
            fill(window, gx1, y(c.right.0) - 1.0, widths[1], 2.0, color);
        }
    }
}

// Keep the benchmark's established CLI test filter while exercising the extracted model.
impl MergeState {

    pub(super) fn scroll_horizontal(&mut self, pane:usize, delta:f32, char_w:f32){
        if self.horizontal_version!=(self.version,self.base_shown){self.columns=[None;3];self.horizontal_version=(self.version,self.base_shown);}
        if delta<=0.0&&self.horizontal[pane]==0.0{return;}
        let lines=match pane{0=>self.core.m.side(Some(Pick::Ours)),1=>self.core.mid_lines(),_=>self.core.m.side(Some(Pick::Theirs))};
        let widest=*self.columns[pane].get_or_insert_with(||super::horizontal::columns(lines));
        let width=(self.pane_w[pane]-num_w()-16.0).max(0.0);
        self.horizontal[pane]=(self.horizontal[pane]+delta).clamp(0.0,(widest as f32*char_w-width).max(0.0));
    }
}


#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn merge_view_perf() {
        crate::merge_session::tests::merge_view_perf();
    }
}

#[cfg(all(test, feature = "gui-test"))]
pub(crate) mod gui_tests {
    use super::*;
    use gpui::{AppContext, Entity, Keystroke, Modifiers, TestAppContext};
    pub(crate) fn event(key: &str, control: bool, shift: bool, alt: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: Keystroke {
                key: key.into(),
                key_char: (key.chars().count() == 1).then(|| key.into()),
                modifiers: Modifiers {
                    control,
                    shift,
                    alt,
                    ..Default::default()
                },
            },
            is_held: false,
            prefer_character_input: false,
        }
    }
    pub(crate) fn fixture(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Viewer>) {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for (name, text) in [
            ("base", "needle base\n"),
            ("ours", "needle ours\n"),
            ("theirs", "needle theirs\n"),
            ("result", "needle ours\n"),
        ] {
            std::fs::write(d.join(name), text).unwrap();
        }
        let spec = Spec::Plain {
            left: d.join("ours"),
            right: d.join("theirs"),
        };
        let set = Arc::new(ChangeSet::load(&spec, d).unwrap());
        let doc = Arc::new(Doc::new(
            "needle ours\n",
            "needle theirs\n",
            &d.join("ours"),
            &d.join("theirs"),
        ));
        let opened = open_files(
            &d.join("base"),
            &d.join("ours"),
            &d.join("theirs"),
            &d.join("result"),
            d,
        )
        .unwrap();
        let viewer = cx.new(|cx| {
            let mut viewer = Viewer::new(
                set,
                spec,
                false,
                None,
                doc,
                View::new(1, 1, &[], 20.0),
                false,
                Options::default(),
                gpui::font("monospace"),
                Instant::now(),
                false,
                cx,
            );
            viewer.start_merge(opened, false, cx);
            viewer
        });
        (dir, viewer)
    }
    #[test]
    fn gui_merge_real_keys_accept_all_four_choices_undo_and_save_exact_result() {
        for (key, expected) in [
            ("right", "needle ours\n"),
            ("left", "needle theirs\n"),
            ("down", "needle ours\nneedle theirs\n"),
            ("backspace", "needle base\n"),
        ] {
            let mut cx = TestAppContext::single();
            cx.skip_drawing();
            let (dir, viewer) = fixture(&mut cx);
            viewer.update(&mut cx, |v, cx| {
                assert!(v.merge_key(&event(key, false, true, true), cx))
            });
            assert_eq!(
                viewer.read_with(&cx, |v, _| v.merge.as_ref().unwrap().text()),
                expected
            );
            viewer.update(&mut cx, |v, cx| {
                assert!(v.merge_key(&event("z", true, false, false), cx))
            });
            assert_eq!(
                viewer.read_with(&cx, |v, _| v.merge.as_ref().unwrap().m.unresolved()),
                1
            );
            viewer.update(&mut cx, |v, cx| {
                v.merge_key(&event("y", true, false, false), cx);
                v.merge_key(&event("s", true, false, false), cx);
            });
            assert_eq!(
                std::fs::read_to_string(dir.path().join("result")).unwrap(),
                expected
            );
            viewer.update(&mut cx, |v, _| {
                let frame = v.merge_frame(12.0, [300.0;3]).unwrap();
                assert!(frame.headers[1].contains("Result"));
                assert!(!frame.panes[1].is_empty());
            });
        }
    }
    #[test]
    fn gui_merge_find_input_produces_visible_matches_and_shared_menu_labels() {
        let mut cx = TestAppContext::single();
        cx.skip_drawing();
        let (_dir, viewer) = fixture(&mut cx);
        viewer.update(&mut cx, |v, cx| {
            v.merge_key(&event("f", true, false, false), cx);
            for c in "needle".chars() {
                v.merge_key(&event(&c.to_string(), false, false, false), cx);
            }
        });
        viewer.update(&mut cx, |v, _| {
            let frame = v.merge_frame(12.0, [300.0;3]).unwrap();
            assert!(
                frame
                    .ask
                    .as_ref()
                    .unwrap()
                    .0
                    .contains("Find Result: needle")
            );
            assert!(
                frame.panes[1]
                    .iter()
                    .any(|row| row.line.as_ref().is_some_and(|l| l.bg.iter().any(|(_, color)| *color == v.palette.found_bg))),
                "visible GUI result rows must show find highlights"
            );
        });
        viewer.update(&mut cx, |v, cx| {
            v.merge_key(&event("enter", false, false, false), cx);
            v.merge_key(&event("escape", false, false, false), cx);
            v.merge_key(&event(":", false, false, false), cx);
        });
        assert!(viewer.read_with(&cx, |v, _| {
            v.command_palette.as_ref().is_some_and(|p|p.rows().any(|r|r.entry.action==crate::catalog::Action::Merge(Act::ResolveSimple)))
        }));
    }

    #[test]
    fn gui_merge_no_current_choice_notifies_toast_before_expiry() {
        let mut cx = TestAppContext::single();
        cx.skip_drawing();
        let (_dir, viewer) = fixture(&mut cx);
        viewer.update(&mut cx, |v, _| {
            let merge = v.merge.as_mut().unwrap();
            merge.control(Control::KeepBase(0));
            merge.current = None;
            assert_eq!(merge.m.unresolved(), 0);
        });
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone()))
            .unwrap();
        let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = notifications.clone();
        let _subscription =
            cx.update(|cx| cx.observe(&viewer, move |_, _| count.set(count.get() + 1)));
        window
            .update(&mut cx, |v, w, cx| {
                v.on_key(&event("right", false, true, true), w, cx)
            })
            .unwrap();
        // Do not run the executor, advance its clock or render: expiry cannot mask missing input notification.
        assert!(
            notifications.get() > 0,
            "No change here must notify immediately instead of waiting for the toast's 1600ms timer"
        );
        assert!(viewer.read_with(&cx, |v, _| {
            let (message, at) = v.toast.as_ref().unwrap();
            message == "No change here" && at.elapsed() < TOAST_FOR
        }));
    }
}
