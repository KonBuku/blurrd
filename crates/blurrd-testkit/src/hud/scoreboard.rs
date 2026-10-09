//! The scoreboard (tab) screen.
//!
//! Geometry in reference pixels, measured on real recordings (one 5v5
//! scoreboard, centre-anchored): a panel from x 572 to 1348 with a dark
//! header band at y 309..338 (column titles), then one row per player on
//! a 34 px pitch (33 px band plus a 1 px dark separator). Each row is
//! tinted in its team's colour and holds the agent icon at x 573..609,
//! the name from x 644 (10 px capitals, baseline 22 px below the row
//! top, descenders to + 25), the ultimate points, kills / deaths /
//! assists, the loadout, the credits and the ping. In a 5v5 game the
//! allied block starts at y 338 and the enemy block at y 569, with the
//! round-history strip (61 px) between them; the recording player's row
//! has a thin outline, and dead players' names are drawn dim grey. A
//! deathmatch scoreboard (no recording available, decision D5) is drawn
//! as one block of up to [`MAX_ROWS`] rows without the strip.

use blurrd_core::{NameKind, RectI, Region};

use super::killfeed::Team;
use super::screen::{ScreenTiming, outline};
use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter, rect_f};
use crate::palette::{Palette, TEXT_GREY, TEXT_WHITE};
use crate::truth::{ContentKind, ItemId};

/// Left edge of the panel.
pub const PANEL_X0: i32 = 572;
/// Right edge of the panel.
pub const PANEL_X1: i32 = 1348;
/// Top of the header band.
pub const HEADER_TOP: i32 = 309;
/// Top of the first row (bottom of the header band).
pub const FIRST_ROW_TOP: i32 = 338;
/// Distance between row tops.
pub const ROW_PITCH: i32 = 34;
/// Height of the coloured band of a row (the separator follows).
pub const ROW_H: i32 = 33;
/// Height of the round-history strip between the two teams.
pub const HISTORY_H: i32 = 61;
/// Agent icon column.
pub const ICON_X: (i32, i32) = (573, 609);
/// Left edge of the name's pen.
pub const NAME_X: f32 = 644.0;
/// Capital height of the names.
pub const NAME_CAP: f32 = 10.0;
/// Baseline of the names below the row top.
pub const BASELINE: f32 = 22.0;
/// The name cell: the slot box of every name (x range; the y range is
/// the row band).
pub const NAME_CELL_X: (i32, i32) = (636, 838);
/// Most rows a scoreboard can show (a deathmatch lobby).
pub const MAX_ROWS: usize = 14;

/// Item ids of scoreboard names start here.
const ID_BASE: u32 = 5000;
/// Dim colour of dead players' names.
const DEAD_NAME: [u8; 3] = [150, 154, 156];
/// Row tint of a deathmatch scoreboard (no teams).
const NEUTRAL_TINT: [u8; 3] = [120, 124, 130];

/// One player row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreRow {
    /// The player's name.
    pub name: String,
    /// The team (ignored in deathmatch).
    pub team: Team,
    /// Whether this is the recording player's own row (outlined).
    pub own: bool,
    /// Whether the player is dead (name drawn dim).
    pub dead: bool,
}

/// How the rows are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScoreboardMode {
    /// Two teams: allied rows, the round-history strip, enemy rows.
    Teams,
    /// One block of rows without teams.
    Deathmatch,
}

/// A scoreboard shown over some frames.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreboardSpec {
    /// The rows; in [`ScoreboardMode::Teams`] the allied rows are drawn
    /// first, whatever their order here.
    pub rows: Vec<ScoreRow>,
    /// Row arrangement.
    pub mode: ScoreboardMode,
    /// When the scoreboard is visible.
    pub timing: ScreenTiming,
}

impl ScoreboardSpec {
    /// The reference top of every drawn row, in drawing order, with the
    /// row's index in [`ScoreboardSpec::rows`].
    pub fn row_tops(&self) -> Vec<(i32, usize)> {
        let mut out = Vec::with_capacity(self.rows.len());
        match self.mode {
            ScoreboardMode::Deathmatch => {
                for (k, i) in (0..).zip(0..self.rows.len().min(MAX_ROWS)) {
                    out.push((FIRST_ROW_TOP + ROW_PITCH * k, i));
                }
            }
            ScoreboardMode::Teams => {
                let mut top = FIRST_ROW_TOP;
                for team in [Team::Ally, Team::Enemy] {
                    for (i, row) in self.rows.iter().enumerate() {
                        if row.team == team {
                            out.push((top, i));
                            top += ROW_PITCH;
                        }
                    }
                    // The round-history strip follows the allied block.
                    top += HISTORY_H;
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RowLabels {
    name: Label,
    stats: Vec<(f32, Label)>,
}

/// The scoreboard rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Scoreboard {
    spec: ScoreboardSpec,
    headers: Vec<(f32, Label)>,
    rows: Vec<RowLabels>,
}

impl Scoreboard {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: ScoreboardSpec) -> Self {
        let headers = [
            (664.0, "NAME"),
            (873.0, "ULTIMATE"),
            (993.0, "KDA"),
            (1128.0, "LOADOUT"),
            (1248.0, "CREDS"),
            (1323.0, "PING"),
        ]
        .into_iter()
        .map(|(x, text)| (x, Label::new(fonts, layout, text, 6.0)))
        .collect();
        let rows = (0u32..)
            .zip(&spec.rows)
            .map(|(i, row)| {
                let stats = [
                    (873.0, format!("{} / {}", i % 4, 6 + i % 3)),
                    (953.0, format!("{}", (i * 7) % 13)),
                    (993.0, format!("{}", (i * 3) % 9)),
                    (1033.0, format!("{}", i % 5)),
                    (1250.0, format!("{},{}00", 1 + i % 4, i % 10)),
                    (1323.0, format!("{}", 16 + (i * 11) % 60)),
                ]
                .into_iter()
                .map(|(x, text)| (x, Label::new(fonts, layout, &text, 9.0)))
                .collect();
                RowLabels {
                    name: Label::new(fonts, layout, &row.name, NAME_CAP),
                    stats,
                }
            })
            .collect();
        Self {
            spec,
            headers,
            rows,
        }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, palette: &Palette, frame: u32) {
        let Some((alpha, offset)) = self.spec.timing.phase(frame) else {
            return;
        };
        let a = Anchor::Center;
        let dy = offset.round() as i32;
        let band = |y0: i32, y1: i32| RectI {
            x0: PANEL_X0,
            y0: y0 + dy,
            x1: PANEL_X1,
            y1: y1 + dy,
        };
        p.fill(
            band(HEADER_TOP, FIRST_ROW_TOP),
            a,
            [22, 26, 30],
            0.8 * alpha,
        );
        for (x, label) in &self.headers {
            let pen = (x - label.width * 0.5, (HEADER_TOP + 17 + dy) as f32);
            p.label(
                ContentKind::ScreenText,
                label,
                pen,
                a,
                TEXT_GREY,
                alpha,
                None,
            );
        }
        let tops = self.spec.row_tops();
        let mut bottom = FIRST_ROW_TOP;
        for (top, row) in &tops {
            let row = &self.spec.rows[*row];
            let tint = match (self.spec.mode, row.team) {
                (ScoreboardMode::Deathmatch, _) => NEUTRAL_TINT,
                (ScoreboardMode::Teams, team) => team.colour(palette),
            };
            p.fill(band(*top, top + ROW_H), a, tint, 0.55 * alpha);
            p.fill(
                band(top + ROW_H, top + ROW_PITCH),
                a,
                [12, 16, 18],
                0.6 * alpha,
            );
            if row.own {
                outline(p, band(*top, top + ROW_H), a, palette.own_outline, alpha);
            }
            bottom = bottom.max(top + ROW_PITCH);
        }
        if self.spec.mode == ScoreboardMode::Teams {
            // The round-history strip between the two blocks.
            let first_enemy = tops
                .iter()
                .find(|(_, i)| self.spec.rows[*i].team == Team::Enemy)
                .map(|(t, _)| *t);
            if let Some(enemy_top) = first_enemy {
                paint_history(p, band(enemy_top - HISTORY_H, enemy_top), alpha);
            }
        }
        p.fill(band(bottom, bottom + 18), a, [22, 26, 30], 0.8 * alpha);
        for &(top, i) in &tops {
            self.paint_row_content(p, i, top + dy, alpha);
        }
    }

    /// The agent icon, name and numbers of row `i`, whose top is at `y`.
    fn paint_row_content(&self, p: &mut Painter<'_>, i: usize, y: i32, alpha: f32) {
        let a = Anchor::Center;
        let row = &self.spec.rows[i];
        let labels = &self.rows[i];
        p.portrait(
            RectI {
                x0: ICON_X.0,
                y0: y + 1,
                x1: ICON_X.1,
                y1: y + ROW_H,
            },
            a,
            u64::from(ID_BASE) + i as u64 * 7919,
            alpha,
        );
        let (rgb, name_alpha) = if row.dead {
            (DEAD_NAME, 0.75 * alpha)
        } else {
            (TEXT_WHITE, alpha)
        };
        p.name(
            NameDraw {
                id: ItemId(ID_BASE + i as u32),
                region: Region::Scoreboard,
                kind: NameKind::ScoreboardRow,
                slot: RectI {
                    x0: NAME_CELL_X.0,
                    y0: y + 2,
                    x1: NAME_CELL_X.1,
                    y1: y + ROW_H - 1,
                },
            },
            &labels.name,
            (NAME_X, y as f32 + BASELINE),
            a,
            rgb,
            name_alpha,
            Some(RectI {
                x0: NAME_CELL_X.0,
                y0: y,
                x1: NAME_CELL_X.1,
                y1: y + ROW_H,
            }),
        );
        for (x, label) in &labels.stats {
            let pen = (x - label.width * 0.5, y as f32 + BASELINE);
            p.label(ContentKind::Stat, label, pen, a, TEXT_WHITE, alpha, None);
        }
    }
}

/// The round-history strip `strip` between the two teams: a dark band
/// with two rows of round markers.
fn paint_history(p: &mut Painter<'_>, strip: RectI, alpha: f32) {
    let a = Anchor::Center;
    p.fill(strip, a, [22, 26, 30], 0.6 * alpha);
    for i in 0..24 {
        let x = (700 + 25 * i) as f32;
        for y in [strip.y0 + 16, strip.y0 + 46] {
            let y = y as f32;
            p.fill(rect_f(x, y, x + 3.0, y + 3.0), a, [200; 3], 0.6 * alpha);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{FIRST_ROW_TOP, ScoreRow, ScoreboardMode, ScoreboardSpec};
    use crate::hud::killfeed::Team;
    use crate::hud::screen::ScreenTiming;

    fn row(name: &str, team: Team) -> ScoreRow {
        ScoreRow {
            name: name.to_owned(),
            team,
            own: false,
            dead: false,
        }
    }

    #[test]
    fn team_rows_match_the_measured_grid() {
        let mut rows: Vec<_> = (0..5).map(|i| row(&format!("e{i}"), Team::Enemy)).collect();
        rows.extend((0..5).map(|i| row(&format!("a{i}"), Team::Ally)));
        let spec = ScoreboardSpec {
            rows,
            mode: ScoreboardMode::Teams,
            timing: ScreenTiming::instant(0, 10),
        };
        let tops: Vec<i32> = spec.row_tops().iter().map(|(t, _)| *t).collect();
        // Measured: allies at 338 + 34k, enemies at 569 + 34k.
        assert_eq!(tops, [338, 372, 406, 440, 474, 569, 603, 637, 671, 705]);
        assert!(spec.rows[spec.row_tops()[0].1].name.starts_with('a'));
        assert_eq!(tops[0], FIRST_ROW_TOP);
    }
}
