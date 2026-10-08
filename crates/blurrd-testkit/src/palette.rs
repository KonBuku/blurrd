//! Team and text colours of the synthetic HUD.
//!
//! [`Palette::DEFAULT`] uses colours measured on real recordings (port spec
//! section 0: teal ally bars around `#64BAA2`, red enemy bars around
//! `#E6625D`). The game's colour-blind modes change these colours, but no
//! recording of them is available (decision D5), so the three colour-blind
//! palettes are deliberate stand-ins that move both team colours far away
//! from the teal and red hue ranges. They test that detection does not
//! depend on the default hues, not that it matches the real modes.

/// The colours of one HUD colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Palette {
    /// Short name, used in test output.
    pub name: &'static str,
    /// Kill-feed bar and name colour of the recording player's team.
    pub ally: [u8; 3],
    /// Kill-feed bar and name colour of the other team.
    pub enemy: [u8; 3],
    /// Outline around the recording player's own kill-feed rows.
    pub own_outline: [u8; 3],
    /// Chat colour of team-channel messages.
    pub chat_team: [u8; 3],
    /// Chat colour of broadcast messages.
    pub chat_broadcast: [u8; 3],
}

impl Palette {
    /// The default colours, measured on real recordings.
    pub const DEFAULT: Self = Self {
        name: "default",
        ally: [100, 186, 162],
        enemy: [230, 98, 93],
        own_outline: [240, 220, 90],
        chat_team: [150, 172, 205],
        chat_broadcast: [182, 212, 255],
    };

    /// Stand-in for a deuteranopia mode: blue allies, orange enemies.
    pub const DEUTERANOPIA: Self = Self {
        name: "deuteranopia",
        ally: [70, 130, 230],
        enemy: [236, 160, 50],
        ..Self::DEFAULT
    };

    /// Stand-in for a protanopia mode: sky-blue allies, yellow enemies.
    pub const PROTANOPIA: Self = Self {
        name: "protanopia",
        ally: [60, 160, 230],
        enemy: [222, 210, 70],
        ..Self::DEFAULT
    };

    /// Stand-in for a tritanopia mode: green allies, magenta enemies.
    pub const TRITANOPIA: Self = Self {
        name: "tritanopia",
        ally: [90, 200, 90],
        enemy: [222, 80, 170],
        ..Self::DEFAULT
    };

    /// Every palette.
    pub const ALL: [Self; 4] = [
        Self::DEFAULT,
        Self::DEUTERANOPIA,
        Self::PROTANOPIA,
        Self::TRITANOPIA,
    ];
}

/// The colour of HUD text (names, messages).
pub const TEXT_WHITE: [u8; 3] = [245, 245, 245];
/// The colour of secondary HUD text (voice location lines).
pub const TEXT_GREY: [u8; 3] = [178, 178, 178];
/// The colour of pure white HUD icons (weapons).
pub const ICON_WHITE: [u8; 3] = [250, 250, 250];

#[cfg(test)]
mod tests {
    use super::Palette;

    /// Hue in degrees of an RGB colour.
    fn hue(c: [u8; 3]) -> f32 {
        let [r, g, b] = c.map(f32::from);
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        let d = mx - mn;
        let h = if (mx - r).abs() < f32::EPSILON {
            60.0 * (g - b) / d
        } else if (mx - g).abs() < f32::EPSILON {
            120.0 + 60.0 * (b - r) / d
        } else {
            240.0 + 60.0 * (r - g) / d
        };
        h.rem_euclid(360.0)
    }

    #[test]
    fn colour_blind_palettes_leave_the_default_hue_ranges() {
        // Default ranges (port spec section 2): teal 132..192 degrees, red
        // up to 20 or from 340 degrees.
        let teal = |h: f32| (132.0..=192.0).contains(&h);
        let red = |h: f32| h <= 20.0 || h >= 340.0;
        assert!(teal(hue(Palette::DEFAULT.ally)) && red(hue(Palette::DEFAULT.enemy)));
        for p in &Palette::ALL[1..] {
            for c in [p.ally, p.enemy] {
                assert!(!teal(hue(c)) && !red(hue(c)), "{} {c:?}", p.name);
            }
        }
    }
}
