//! Kill-feed rows in the top-right corner.
//!
//! Geometry in reference pixels from the port spec (section 3): rows are
//! 33 px tall on a 39 px pitch with the first row's top at y 96, right
//! aligned at x 1900; the victim portrait spans x 1838..1900 and the victim
//! name ends near x 1832; names have 14 px capitals with the baseline at
//! the row top + 24. The newest row is at the bottom; when the oldest row
//! has faded out, every row below jumps up by one pitch.
//!
//! Animations per row: a one-frame cover bar without content, then the
//! content slides in from the left (60, 25 and 8 px short of its place),
//! and at the end of its life the row fades out.

use blurrd_core::{NameKind, RectI, Region};

use crate::font::FontSet;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter, rect_f};
use crate::palette::{ICON_WHITE, Palette, TEXT_WHITE};
use crate::truth::{ContentKind, ItemId};

/// Top of the first row.
pub const FIRST_ROW_TOP: i32 = 96;
/// Row height.
pub const ROW_H: i32 = 33;
/// Distance between row tops.
pub const ROW_PITCH: i32 = 39;
/// Right edge of every row.
pub const ROW_RIGHT: i32 = 1900;
/// Left edge of the victim portrait.
pub const VICTIM_PORTRAIT_X0: i32 = 1838;
/// Right edge of the victim name's pen advance.
pub const VICTIM_TEXT_X1: f32 = 1832.0;
/// Capital height of the names.
pub const NAME_CAP: f32 = 14.0;
/// Baseline of the names below the row top.
pub const BASELINE: f32 = 24.0;
/// Horizontal offsets of the content in the frames after the cover bar.
pub const SLIDE_IN: [f32; 3] = [-60.0, -25.0, -8.0];

/// Item ids of kill-feed names start here (killer `2k`, victim `2k + 1`).
const ID_BASE: u32 = 1000;

/// Which team a player is on, from the recording player's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    /// The recording player's team.
    Ally,
    /// The other team.
    Enemy,
}

impl Team {
    /// The bar colour of the team in a palette.
    pub const fn colour(self, palette: &Palette) -> [u8; 3] {
        match self {
            Self::Ally => palette.ally,
            Self::Enemy => palette.enemy,
        }
    }
}

/// What the killer used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weapon {
    /// A long white gun silhouette.
    Rifle,
    /// A short white gun silhouette.
    Pistol,
    /// A knife: a thin sliver too small to count as a weapon blob.
    Knife,
    /// A small ability icon, also too small for a weapon blob.
    Ability,
}

impl Weapon {
    /// Width and height of the icon in reference pixels.
    const fn size(self) -> (i32, i32) {
        match self {
            Self::Rifle => (92, 16),
            Self::Pistol => (40, 15),
            Self::Knife => (34, 7),
            Self::Ability => (11, 16),
        }
    }
}

/// One kill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kill {
    /// Killer name.
    pub killer: String,
    /// Victim name.
    pub victim: String,
    /// Team of the killer (the left half of the row).
    pub killer_team: Team,
    /// Team of the victim (the right half of the row). The same team as the
    /// killer gives a one-colour row with a round icon between the names,
    /// like self, ability or spike kills.
    pub victim_team: Team,
    /// The killer's weapon (ignored in one-colour rows).
    pub weapon: Weapon,
    /// Multikill count shown in a badge in front of the row (2 to 5).
    pub badge: Option<u8>,
    /// Whether the recording player is involved (yellow outline).
    pub own: bool,
    /// Whether a headshot icon follows the weapon.
    pub headshot: bool,
    /// Frame in which the row appears.
    pub birth: u32,
}

/// The kill feed of a scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillFeedSpec {
    /// The kills, in any order.
    pub kills: Vec<Kill>,
    /// Frames a row stays fully visible.
    pub life: u32,
    /// Frames a row takes to fade out after its life.
    pub fade: u32,
    /// Whether rows animate in (cover bar and slide); otherwise they appear
    /// complete in their first frame.
    pub wipe_in: bool,
}

impl KillFeedSpec {
    /// Rows stay 5 s (at 60 fps) and fade over 18 frames.
    pub fn new(kills: Vec<Kill>) -> Self {
        Self {
            kills,
            life: 300,
            fade: 18,
            wipe_in: true,
        }
    }
}

/// Reference geometry of one row, relative to its top `y0` (x absolute).
#[derive(Debug, Clone, PartialEq)]
struct RowGeometry {
    killer_portrait: RectI,
    killer_pen: f32,
    victim_pen: f32,
    boundary: i32,
    bar_x0: i32,
    icon: Option<RectI>,
    headshot: Option<RectI>,
    badge: Option<RectI>,
    killer_slot: (i32, i32),
    victim_slot: (i32, i32),
}

/// A kill rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    kill: Kill,
    index: u32,
    killer: Label,
    victim: Label,
    badge: Option<Label>,
    geometry: RowGeometry,
}

impl Row {
    fn new(fonts: &FontSet, layout: &Layout, kill: Kill, index: u32) -> Self {
        let killer = Label::new(fonts, layout, &kill.killer, NAME_CAP);
        let victim = Label::new(fonts, layout, &kill.victim, NAME_CAP);
        let badge = kill.badge.map(|n| Label::new(fonts, layout, roman(n), 9.0));
        let geometry = geometry(&kill, killer.width, victim.width);
        Self {
            kill,
            index,
            killer,
            victim,
            badge,
            geometry,
        }
    }

    fn one_colour(&self) -> bool {
        self.kill.killer_team == self.kill.victim_team
    }
}

fn roman(n: u8) -> &'static str {
    match n {
        0..=2 => "II",
        3 => "III",
        4 => "IV",
        _ => "V",
    }
}

fn geometry(kill: &Kill, killer_w: f32, victim_w: f32) -> RowGeometry {
    let victim_x0 = (VICTIM_TEXT_X1 - victim_w).floor() as i32;
    let one_colour = kill.killer_team == kill.victim_team;
    let (boundary, icon, headshot, killer_x1) = if one_colour {
        // NAME [round icon] NAME, one bar colour.
        let icon = RectI {
            x0: victim_x0 - 34,
            y0: 5,
            x1: victim_x0 - 10,
            y1: 29,
        };
        (ROW_RIGHT, Some(icon), None, icon.x0 - 10)
    } else {
        let boundary = victim_x0 - 10;
        let headshot = kill.headshot.then_some(RectI {
            x0: boundary - 28,
            y0: 7,
            x1: boundary - 8,
            y1: 27,
        });
        let (w, h) = kill.weapon.size();
        let wx1 = if kill.headshot {
            boundary - 40
        } else {
            boundary - 14
        };
        let top = (ROW_H - h) / 2;
        let icon = RectI {
            x0: wx1 - w,
            y0: top,
            x1: wx1,
            y1: top + h,
        };
        (boundary, Some(icon), headshot, icon.x0 - 12)
    };
    let killer_x0 = killer_x1 as f32 - killer_w;
    let portrait_x1 = killer_x0.floor() as i32 - 8;
    let killer_portrait = RectI {
        x0: portrait_x1 - 62,
        y0: 0,
        x1: portrait_x1,
        y1: ROW_H,
    };
    let badge = kill.badge.map(|_| RectI {
        x0: killer_portrait.x0 - 30,
        y0: 3,
        x1: killer_portrait.x0 - 4,
        y1: 29,
    });
    let icon_x0 = icon.map_or(killer_x1 + 12, |i| i.x0);
    let icon_x1 = icon.map_or(victim_x0 - 10, |i| i.x1);
    RowGeometry {
        killer_portrait,
        killer_pen: killer_x0,
        victim_pen: VICTIM_TEXT_X1 - victim_w,
        boundary,
        bar_x0: killer_portrait.x0,
        icon,
        headshot,
        badge,
        killer_slot: (portrait_x1, icon_x0 - 2),
        victim_slot: (
            if one_colour { icon_x1 } else { boundary + 2 },
            VICTIM_PORTRAIT_X0 - 2,
        ),
    }
}

/// The kill feed rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KillFeed {
    spec: KillFeedSpec,
    rows: Vec<Row>,
}

impl KillFeed {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: KillFeedSpec) -> Self {
        let mut kills: Vec<(u32, Kill)> = (0..).zip(spec.kills.iter().cloned()).collect();
        kills.sort_by_key(|(i, k)| (k.birth, *i));
        let rows = kills
            .into_iter()
            .map(|(i, k)| Row::new(fonts, layout, k, i))
            .collect();
        Self { spec, rows }
    }

    /// Paints the rows visible in `frame`.
    pub(crate) fn paint(&self, p: &mut Painter<'_>, palette: &Palette, frame: u32) {
        let end = self.spec.life + self.spec.fade;
        let alive = self
            .rows
            .iter()
            .filter(|r| frame >= r.kill.birth && frame - r.kill.birth < end);
        for (slot, row) in (0..).zip(alive) {
            let age = frame - row.kill.birth;
            let alpha = if age < self.spec.life {
                1.0
            } else {
                1.0 - (age - self.spec.life + 1) as f32 / (self.spec.fade + 1) as f32
            };
            let y0 = FIRST_ROW_TOP + ROW_PITCH * slot;
            self.paint_row(p, palette, row, y0, age, alpha);
        }
    }

    fn paint_row(
        &self,
        p: &mut Painter<'_>,
        palette: &Palette,
        row: &Row,
        y0: i32,
        age: u32,
        alpha: f32,
    ) {
        let g = &row.geometry;
        let a = Anchor::Right;
        let killer_colour = row.kill.killer_team.colour(palette);
        let band = |x0: i32, x1: i32| band(x0, x1, y0);
        paint_bars(p, palette, row, y0, alpha);
        // In the cover frame the bar has no content yet.
        if self.spec.wipe_in && age == 0 {
            return;
        }
        let dx = if self.spec.wipe_in {
            SLIDE_IN
                .get((age as usize).wrapping_sub(1))
                .copied()
                .unwrap_or(0.0)
        } else {
            0.0
        };
        let shift = |r: RectI| {
            let d = dx.round() as i32;
            RectI {
                x0: r.x0 + d,
                y0: r.y0 + y0,
                x1: r.x1 + d,
                y1: r.y1 + y0,
            }
        };
        // Content that slides in stays inside the row's bar.
        let clip = Some(band(g.bar_x0, ROW_RIGHT));
        let seed = u64::from(row.index) * 7919;
        p.portrait(shift(g.killer_portrait), a, seed, alpha);
        p.portrait(
            shift(RectI {
                x0: VICTIM_PORTRAIT_X0,
                y0: 0,
                x1: ROW_RIGHT,
                y1: ROW_H,
            }),
            a,
            seed + 1,
            alpha,
        );
        if let (Some(badge), Some(label)) = (g.badge, &row.badge) {
            let r = shift(badge);
            p.rounded(r, a, 13.0, killer_colour, alpha);
            let pen = (
                r.x0 as f32 + (r.width() as f32 - label.width) * 0.5,
                (y0 + 21) as f32,
            );
            p.label(ContentKind::Badge, label, pen, a, TEXT_WHITE, alpha, None);
            p.content(ContentKind::Badge, r, a);
        }
        if let Some(icon) = g.icon {
            let r = shift(icon);
            if row.one_colour() {
                p.rounded(r, a, 12.0, ICON_WHITE, alpha);
            } else {
                weapon(p, r, row.kill.weapon, alpha);
            }
            p.content(ContentKind::Icon, r, a);
        }
        if let Some(hs) = g.headshot {
            let r = shift(hs);
            p.rounded(r, a, 4.0, ICON_WHITE, alpha);
            p.content(ContentKind::Icon, r, a);
        }
        let baseline = y0 as f32 + BASELINE;
        let slot =
            |(x0, x1): (i32, i32)| rect_f(x0 as f32, (y0 + 5) as f32, x1 as f32, (y0 + 28) as f32);
        let id = ID_BASE + 2 * row.index;
        p.name(
            NameDraw {
                id: ItemId(id),
                region: Region::KillFeed,
                kind: NameKind::Killer,
                slot: slot(g.killer_slot),
            },
            &row.killer,
            (g.killer_pen + dx, baseline),
            a,
            TEXT_WHITE,
            alpha,
            clip,
        );
        p.name(
            NameDraw {
                id: ItemId(id + 1),
                region: Region::KillFeed,
                kind: NameKind::Victim,
                slot: slot(g.victim_slot),
            },
            &row.victim,
            (g.victim_pen + dx, baseline),
            a,
            TEXT_WHITE,
            alpha,
            clip,
        );
    }
}

/// The reference band of a row from `x0` to `x1`.
fn band(x0: i32, x1: i32, y0: i32) -> RectI {
    RectI {
        x0,
        y0,
        x1: x1.max(x0 + 1),
        y1: y0 + ROW_H,
    }
}

/// Bars of a row: the killer half, then the victim half (one colour for
/// single-team rows), and the outline of the recording player's rows.
fn paint_bars(p: &mut Painter<'_>, palette: &Palette, row: &Row, y0: i32, alpha: f32) {
    let g = &row.geometry;
    let a = Anchor::Right;
    let killer = row.kill.killer_team.colour(palette);
    let victim = row.kill.victim_team.colour(palette);
    p.fill(band(g.bar_x0, g.boundary, y0), a, killer, 0.92 * alpha);
    if g.boundary < ROW_RIGHT {
        p.fill(band(g.boundary, ROW_RIGHT, y0), a, victim, 0.92 * alpha);
    }
    if row.kill.own {
        outline(p, band(g.bar_x0, ROW_RIGHT, y0), palette.own_outline, alpha);
    }
}

/// A 2 px outline around a reference rectangle.
fn outline(p: &mut Painter<'_>, r: RectI, rgb: [u8; 3], alpha: f32) {
    let a = Anchor::Right;
    for edge in [
        RectI { y1: r.y0 + 2, ..r },
        RectI { y0: r.y1 - 2, ..r },
        RectI { x1: r.x0 + 2, ..r },
        RectI { x0: r.x1 - 2, ..r },
    ] {
        p.fill(edge, a, rgb, alpha);
    }
}

/// A white gun-like silhouette filling `r` (reference pixels).
fn weapon(p: &mut Painter<'_>, r: RectI, weapon: Weapon, alpha: f32) {
    let a = Anchor::Right;
    let (w, h) = (r.width() as i32, r.height() as i32);
    if matches!(weapon, Weapon::Knife | Weapon::Ability) {
        p.fill(r, a, ICON_WHITE, alpha);
        return;
    }
    let mid = r.y0 + h / 3;
    let parts = [
        // Barrel and body.
        RectI {
            x0: r.x0,
            y0: mid,
            x1: r.x1,
            y1: mid + h / 3,
        },
        // Stock (right, the gun points left).
        RectI {
            x0: r.x1 - w / 5,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1 - h / 4,
        },
        // Grip.
        RectI {
            x0: r.x0 + w / 2,
            y0: mid,
            x1: r.x0 + w / 2 + w / 8 + 2,
            y1: r.y1,
        },
    ];
    for part in parts {
        if part.x1 > part.x0 && part.y1 > part.y0 {
            p.fill(part, a, ICON_WHITE, alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kill, Team, Weapon, geometry};

    fn kill(killer_team: Team, victim_team: Team, headshot: bool) -> Kill {
        Kill {
            killer: "Killer".into(),
            victim: "Victim".into(),
            killer_team,
            victim_team,
            weapon: Weapon::Rifle,
            badge: Some(3),
            own: false,
            headshot,
            birth: 0,
        }
    }

    #[test]
    fn lays_out_two_colour_rows_right_to_left() {
        let g = geometry(&kill(Team::Ally, Team::Enemy, true), 50.0, 40.0);
        let icon = g.icon.unwrap();
        let hs = g.headshot.unwrap();
        assert!(g.victim_pen > g.boundary as f32);
        assert!(hs.x1 <= g.boundary && icon.x1 <= hs.x0);
        // Weapon blob rule of the port spec (3.3 step 7).
        assert!(icon.x0 < g.boundary - 36 && icon.x1 >= g.boundary - 80);
        assert!(g.killer_pen + 50.0 <= icon.x0 as f32);
        assert!(g.killer_portrait.x1 <= g.killer_pen as i32);
        assert!(g.badge.unwrap().x1 <= g.killer_portrait.x0);
        assert!(g.killer_slot.0 < g.killer_slot.1 && g.victim_slot.0 < g.victim_slot.1);
    }

    #[test]
    fn one_colour_rows_have_a_round_icon_between_the_names() {
        let g = geometry(&kill(Team::Enemy, Team::Enemy, false), 50.0, 40.0);
        let icon = g.icon.unwrap();
        assert_eq!(g.boundary, super::ROW_RIGHT);
        assert!(icon.height() >= 22);
        assert!(g.killer_pen + 50.0 <= icon.x0 as f32 && icon.x1 as f32 <= g.victim_pen);
    }
}
