//! Text chat on the bottom left.
//!
//! Geometry in reference pixels from the port spec (section 6): a dark
//! multiplicative box (× 0.66) at x 26..469, one-line messages on a 29 px
//! grid with the newest line's top at y 1006 (11.5 px capitals, baseline at
//! the line top + 13), up to three messages (four on some recordings: see
//! [`ChatSpec::shown`]). A new message pushes the older ones up at once; a
//! wrapped message adds a continuation line 18 px below its first line,
//! which moves the layout off the grid; old messages fade out together with
//! their part of the box.
//!
//! While the player types ([`ChatSpec::input`]), the chat is open: the
//! history, faded messages included, fills a panel (× 0.66, x 26..457,
//! from y 787) on the same grid, with a white scrollbar thumb to the right
//! of it, above a darker input line (× 0.47, x 26..470, y 1028..1061) that
//! shows the channel and the typed text. Geometry measured on recordings.
//!
//! A line reads `(Prefix) Name: message`: prefix and name in the channel's
//! colour, colon and message in white. System lines read
//! `(System) Name <phrase>` without a colon.
//!
//! The German and Spanish prefixes and system phrases are plausible
//! translations, not verified against the game's localisation (no
//! recordings in those languages are available, decision D5). They give
//! the prefixes different widths, which is what the detector must cope
//! with.

use blurrd_core::{NameKind, RectI, Region};

use crate::font::FontSet;
use crate::hud::killfeed::Team;
use crate::layout::{Anchor, Layout};
use crate::paint::{Label, NameDraw, Painter, rect_f};
use crate::palette::{Palette, TEXT_WHITE};
use crate::truth::{ContentKind, ItemId};

/// Top of the newest (bottom) line.
pub const LAST_LINE_TOP: i32 = 1006;
/// Distance between line tops.
pub const LINE_PITCH: i32 = 29;
/// Offset of a wrapped message's continuation line.
pub const WRAP_PITCH: i32 = 18;
/// Left edge of the text.
pub const TEXT_X: f32 = 38.0;
/// Horizontal extent of the box.
pub const BOX_X: (i32, i32) = (26, 469);
/// Capital height of the text.
pub const CAP: f32 = 11.5;
/// Widest name in reference pixels (sixteen characters).
pub const MAX_NAME_WIDTH: f32 = 140.0;
/// Messages shown at once by default.
pub const MAX_MESSAGES: usize = 3;
/// Top of the open chat's history panel.
pub const PANEL_TOP: i32 = 787;
/// Right edge of the open chat's history panel.
pub const PANEL_X1: i32 = 457;
/// Rows of the open chat's input line.
pub const INPUT_Y: (i32, i32) = (1028, 1061);
/// Right edge of the open chat's input line.
pub const INPUT_X1: i32 = 470;
/// Lines of the open chat whose text starts above this row are hidden.
const PANEL_TEXT_TOP: i32 = PANEL_TOP + 14;
/// The open chat's scrollbar thumb, its dark outline included.
const THUMB: RectI = RectI {
    x0: 459,
    y0: 945,
    x1: 468,
    y1: 1028,
};
/// Item ids of chat names start here.
const ID_BASE: u32 = 4000;

/// A chat channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Channel {
    /// Everyone; the name has the sender's team colour.
    All,
    /// The sender's team.
    Team,
    /// The sender's party.
    Party,
    /// Observers and casters.
    Broadcast,
    /// Game messages about a player.
    System,
}

/// The client language, which changes prefixes and system phrases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Language {
    /// English.
    English,
    /// German.
    German,
    /// Spanish.
    Spanish,
}

impl Language {
    /// Every language.
    pub const ALL: [Self; 3] = [Self::English, Self::German, Self::Spanish];

    /// The prefix of a channel.
    pub const fn prefix(self, channel: Channel) -> &'static str {
        match (self, channel) {
            (Self::English, Channel::All) => "(All)",
            (Self::English | Self::German, Channel::Team) => "(Team)",
            (Self::English, Channel::Party) => "(Party)",
            (Self::English, Channel::Broadcast) => "(Broadcast)",
            (Self::English | Self::German, Channel::System) => "(System)",
            (Self::German, Channel::All) => "(Alle)",
            (Self::German, Channel::Party) => "(Gruppe)",
            (Self::German, Channel::Broadcast) => "(Übertragung)",
            (Self::Spanish, Channel::All) => "(Todos)",
            (Self::Spanish, Channel::Team) => "(Equipo)",
            (Self::Spanish, Channel::Party) => "(Grupo)",
            (Self::Spanish, Channel::Broadcast) => "(Transmisión)",
            (Self::Spanish, Channel::System) => "(Sistema)",
        }
    }

    /// The phrase after the name in a system line.
    pub const fn system_phrase(self) -> &'static str {
        match self {
            Self::English => "has abandoned the match",
            Self::German => "hat das Match verlassen",
            Self::Spanish => "ha abandonado la partida",
        }
    }
}

/// One chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// The channel.
    pub channel: Channel,
    /// The sender (or, for system lines, the player the line is about).
    pub name: String,
    /// The sender's team (colours the name in the all channel).
    pub team: Team,
    /// The message (ignored for system lines).
    pub text: String,
    /// For broadcast lines, the role shown in parentheses after the name.
    pub role: Option<String>,
    /// Whether the message wraps onto a continuation line.
    pub wrap: bool,
    /// Frame in which the message arrives.
    pub time: u32,
}

/// The chat opened for typing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatInput {
    /// First frame the chat is open in.
    pub from: u32,
    /// Frame it closes in.
    pub to: u32,
    /// The channel typed in, shown on the input line ("All:").
    pub channel: Channel,
    /// The typed text (the player's own).
    pub text: String,
}

/// The text chat of a scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSpec {
    /// Client language.
    pub language: Language,
    /// The messages, in any order.
    pub messages: Vec<ChatMessage>,
    /// Frames a message stays fully visible.
    pub life: u32,
    /// Frames a message takes to fade out.
    pub fade: u32,
    /// Messages the closed chat shows at once.
    pub shown: usize,
    /// When the chat is open for typing, if ever.
    pub input: Option<ChatInput>,
}

impl ChatSpec {
    /// Messages stay 10 s (at 60 fps) and fade over 18 frames (0.3 s).
    pub fn new(language: Language, messages: Vec<ChatMessage>) -> Self {
        Self {
            language,
            messages,
            life: 600,
            fade: 18,
            shown: MAX_MESSAGES,
            input: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Line {
    message: ChatMessage,
    index: u32,
    prefix: Label,
    name: Label,
    /// Role in parentheses, colon, message or system phrase: everything
    /// after the name on the first line.
    tail: Vec<(Label, [u8; 3], ContentKind)>,
    continuation: Option<Label>,
}

/// The text chat rendered for one layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Chat {
    spec: ChatSpec,
    lines: Vec<Line>,
    /// The input line of the open chat: channel and typed text.
    input: Option<(Label, Label)>,
    space: f32,
}

impl Chat {
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, spec: ChatSpec) -> Self {
        let label = |text: &str| Label::new(fonts, layout, text, CAP);
        let mut messages: Vec<(u32, ChatMessage)> =
            (0..).zip(spec.messages.iter().cloned()).collect();
        messages.sort_by_key(|(i, m)| (m.time, *i));
        let lines = messages
            .into_iter()
            .map(|(index, message)| {
                let mut tail = Vec::new();
                let mut continuation = None;
                if message.channel == Channel::System {
                    tail.push((
                        label(spec.language.system_phrase()),
                        TEXT_WHITE,
                        ContentKind::SystemPhrase,
                    ));
                } else {
                    if let Some(role) = &message.role {
                        tail.push((label(&format!("({role})")), [0; 3], ContentKind::ChatPrefix));
                    }
                    let (first, rest) = if message.wrap {
                        split_half(&message.text)
                    } else {
                        (message.text.as_str(), "")
                    };
                    tail.push((
                        label(&format!(": {first}")),
                        TEXT_WHITE,
                        ContentKind::ChatMessage,
                    ));
                    if !rest.is_empty() {
                        continuation = Some(label(rest));
                    }
                }
                Line {
                    prefix: label(spec.language.prefix(message.channel)),
                    name: label(&message.name),
                    tail,
                    continuation,
                    message,
                    index,
                }
            })
            .collect();
        let input = spec.input.as_ref().map(|input| {
            let prefix = spec.language.prefix(input.channel);
            let channel = prefix.trim_start_matches('(').trim_end_matches(')');
            (label(&format!("{channel}:")), label(&input.text))
        });
        Self {
            space: label("a b").width - label("ab").width,
            spec,
            lines,
            input,
        }
    }

    pub(crate) fn paint(&self, p: &mut Painter<'_>, palette: &Palette, frame: u32) {
        if let Some(input) = self
            .spec
            .input
            .as_ref()
            .filter(|i| (i.from..i.to).contains(&frame))
        {
            self.paint_open(p, palette, input.channel, frame);
            return;
        }
        let end = self.spec.life + self.spec.fade;
        let visible: Vec<&Line> = self
            .lines
            .iter()
            .filter(|l| frame >= l.message.time && frame - l.message.time < end)
            .collect();
        let start = visible.len().saturating_sub(self.spec.shown);
        let mut last_top = LAST_LINE_TOP;
        for line in visible[start..].iter().rev() {
            let rows = if line.continuation.is_some() { 2 } else { 1 };
            let first_top = last_top - WRAP_PITCH * (rows - 1);
            let age = frame - line.message.time;
            let alpha = if age < self.spec.life {
                1.0
            } else {
                1.0 - (age - self.spec.life + 1) as f32 / (self.spec.fade + 1) as f32
            };
            self.paint_line(p, palette, line, first_top, last_top, Some(alpha));
            last_top = first_top - LINE_PITCH;
        }
    }

    /// Paints the open chat: the whole history on the panel, the scrollbar
    /// and the input line of `channel`.
    fn paint_open(&self, p: &mut Painter<'_>, palette: &Palette, channel: Channel, frame: u32) {
        let a = Anchor::Left;
        let panel = p.map(
            RectI {
                x0: BOX_X.0,
                y0: PANEL_TOP,
                x1: PANEL_X1,
                y1: INPUT_Y.0,
            },
            a,
        );
        p.frame.multiply_rect(panel, 0.66);
        p.fill(THUMB, a, [0; 3], 1.0);
        let thumb = RectI {
            x0: THUMB.x0 + 1,
            y0: THUMB.y0 + 1,
            x1: THUMB.x1 - 1,
            y1: THUMB.y1 - 1,
        };
        p.fill(thumb, a, TEXT_WHITE, 1.0);
        let input_line = p.map(
            RectI {
                x0: BOX_X.0,
                y0: INPUT_Y.0,
                x1: INPUT_X1,
                y1: INPUT_Y.1,
            },
            a,
        );
        p.frame.multiply_rect(input_line, 0.47);
        if let Some((name, typed)) = &self.input {
            let baseline = (INPUT_Y.0 + 22) as f32;
            let colour = channel_colour(channel, Team::Ally, palette);
            p.label(
                ContentKind::ChatPrefix,
                name,
                (TEXT_X, baseline),
                a,
                colour,
                1.0,
                None,
            );
            p.label(
                ContentKind::ChatMessage,
                typed,
                (TEXT_X + name.width + self.space, baseline),
                a,
                TEXT_WHITE,
                1.0,
                None,
            );
        }
        let mut last_top = LAST_LINE_TOP;
        for line in self.lines.iter().rev().filter(|l| l.message.time <= frame) {
            let rows = if line.continuation.is_some() { 2 } else { 1 };
            let first_top = last_top - WRAP_PITCH * (rows - 1);
            if first_top < PANEL_TEXT_TOP {
                break;
            }
            self.paint_line(p, palette, line, first_top, last_top, None);
            last_top = first_top - LINE_PITCH;
        }
    }

    /// Paints one message from row `first_top` to row `last_top` (its
    /// continuation line) with opacity `alpha` over its own part of the
    /// box, or, in the open chat (`alpha` is `None`), fully visible on the
    /// panel.
    fn paint_line(
        &self,
        p: &mut Painter<'_>,
        palette: &Palette,
        line: &Line,
        first_top: i32,
        last_top: i32,
        alpha: Option<f32>,
    ) {
        let a = Anchor::Left;
        if let Some(alpha) = alpha {
            let band = RectI {
                x0: BOX_X.0,
                y0: first_top - 6,
                x1: BOX_X.1,
                y1: last_top + LINE_PITCH - 6,
            };
            let band_src = p.map(band, a);
            p.frame.multiply_rect(band_src, 1.0 - 0.34 * alpha);
        }
        let alpha = alpha.unwrap_or(1.0);
        let colour = channel_colour(line.message.channel, line.message.team, palette);
        let baseline = (first_top + 13) as f32;
        p.label(
            ContentKind::ChatPrefix,
            &line.prefix,
            (TEXT_X, baseline),
            a,
            colour,
            alpha,
            None,
        );
        let name_x = TEXT_X + line.prefix.width + self.space;
        let slot = rect_f(
            name_x - 2.0,
            (first_top - 2) as f32,
            name_x + MAX_NAME_WIDTH + 4.0,
            (first_top + 18) as f32,
        );
        p.name(
            NameDraw {
                id: ItemId(ID_BASE + line.index),
                region: Region::TextChat,
                kind: NameKind::ChatSender,
                slot,
            },
            &line.name,
            (name_x, baseline),
            a,
            colour,
            alpha,
            None,
        );
        let mut x = name_x + line.name.width;
        for (label, rgb, kind) in &line.tail {
            // Roles keep the channel colour; the system phrase and the
            // message are separated from the name by a space, the colon is
            // glued to it.
            let rgb = if *kind == ContentKind::ChatPrefix {
                colour
            } else {
                *rgb
            };
            if *kind != ContentKind::ChatMessage {
                x += self.space;
            }
            p.label(*kind, label, (x, baseline), a, rgb, alpha, None);
            x += label.width;
        }
        if let Some(rest) = &line.continuation {
            p.label(
                ContentKind::ChatMessage,
                rest,
                (TEXT_X, (last_top + 13) as f32),
                a,
                TEXT_WHITE,
                alpha,
                None,
            );
        }
    }
}

/// The colour of a channel's prefix and names; in the all channel the
/// sender's team colour.
fn channel_colour(channel: Channel, team: Team, palette: &Palette) -> [u8; 3] {
    match channel {
        Channel::All => team.colour(palette),
        Channel::Team | Channel::Party => palette.chat_team,
        Channel::Broadcast => palette.chat_broadcast,
        Channel::System => TEXT_WHITE,
    }
}

/// Splits a message at the space nearest to its middle.
fn split_half(text: &str) -> (&str, &str) {
    let mid = text.len() / 2;
    let split = text
        .char_indices()
        .filter(|(_, c)| *c == ' ')
        .map(|(i, _)| i)
        .min_by_key(|i| i.abs_diff(mid));
    match split {
        Some(i) => (&text[..i], &text[i + 1..]),
        None => (text, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::{Channel, Language, split_half};

    #[test]
    fn prefixes_differ_per_language() {
        for channel in [Channel::All, Channel::Broadcast, Channel::System] {
            let p: Vec<_> = Language::ALL.iter().map(|l| l.prefix(channel)).collect();
            assert!(p[0] != p[2], "{channel:?}");
        }
        assert!(
            Language::ALL
                .iter()
                .all(|l| l.prefix(Channel::Team).starts_with('('))
        );
    }

    #[test]
    fn splits_messages_near_the_middle() {
        assert_eq!(split_half("one two three four"), ("one two", "three four"));
        assert_eq!(split_half("single"), ("single", ""));
    }
}
