//! The text-chat detector on synthetic frames from blurrd-testkit: the
//! sender of every chat line is found and fully covered, the message is
//! not.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small frames: coordinates never wrap, a panic is a failure"
)]

mod support;

use blurrd_core::{NameKind, Region};
use blurrd_detect::chat::{ChatDetector, LEFT_PANEL_ROI, NameEnd, NameStart};
use blurrd_detect::{DetectError, Detection, RgbView};
use blurrd_testkit::hud::chat::{Channel, ChatMessage, ChatSpec, Language};
use blurrd_testkit::hud::killfeed::Team;
use blurrd_testkit::metrics::{OverBlur, item_coverage, over_blur};
use blurrd_testkit::{
    Background, ContentKind, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene,
    synth_name,
};

use support::prepared;

/// Backgrounds behind the chat: textured, dark and bright.
const BACKGROUNDS: [Background; 3] = [
    Background::noise(3),
    Background::dusk(),
    Background::white_sky(),
];

/// Single-word names of every length.
const SINGLE_WORD: [NameStyle; 5] = [
    NameStyle::Latin(3),
    NameStyle::Latin(7),
    NameStyle::Latin(11),
    NameStyle::Latin(16),
    NameStyle::Digits(6),
];

/// The [`LEFT_PANEL_ROI`] of a reference-size frame, without copying.
fn left_view(frame: &RgbFrame) -> RgbView<'_> {
    let stride = 3 * frame.width() as usize;
    let start = LEFT_PANEL_ROI.y0 as usize * stride + 3 * LEFT_PANEL_ROI.x0 as usize;
    RgbView::new(
        &frame.data()[start..],
        LEFT_PANEL_ROI.width(),
        LEFT_PANEL_ROI.height(),
        stride,
    )
    .unwrap()
}

/// A message on `channel` with a name of `style`.
fn message(rng: &mut Rng, channel: Channel, style: NameStyle, time: u32) -> ChatMessage {
    ChatMessage {
        channel,
        name: synth_name(rng, style),
        team: *rng.pick(&[Team::Ally, Team::Enemy]),
        text: (*rng.pick(&["nice", "go b long", "rotate mid now", "one hp", "gg wp all"]))
            .to_owned(),
        role: (channel == Channel::Broadcast).then(|| "Sova".to_owned()),
        wrap: false,
        time,
    }
}

/// The frame of a chat where `messages` arrived one frame apart, ten
/// frames after the last one.
fn chat_fixture(
    language: Language,
    messages: Vec<ChatMessage>,
    background: Background,
    palette: Palette,
) -> Fixture {
    let fonts = FontSet::latin();
    let last = messages.iter().map(|m| m.time).max().unwrap_or(0);
    Scene::new(Layout::reference(), background, palette, last + 20)
        .with_chat(&fonts, ChatSpec::new(language, messages))
        .render(last + 10)
}

/// The chat detections of one frame, checked against its truth.
#[derive(Debug, Default)]
struct Check {
    items: usize,
    detections: Vec<Detection>,
    /// Names with glyph pixels the boxes do not fully cover.
    leaks: Vec<String>,
    /// Names no detection overlaps.
    missed: Vec<String>,
    /// Detections that overlap no chat name.
    false_positives: Vec<String>,
    /// Blurred pixels, and those away from every name.
    over_blur: OverBlur,
    /// Whether a line was covered with Safe geometry (start or end of its
    /// name not found).
    safe: bool,
}

impl Check {
    fn assert_clean(&self, context: &str) {
        assert!(
            self.leaks.is_empty() && self.missed.is_empty() && self.false_positives.is_empty(),
            "{context}: leaks {:?}, missed {:?}, false positives {:?}, detections {:?}",
            self.leaks,
            self.missed,
            self.false_positives,
            self.detections
        );
    }
}

/// Runs `detector` on a fixture and checks every chat name.
fn check_with(detector: &mut ChatDetector, fx: &Fixture) -> Check {
    let mut detections = vec![];
    detector
        .detect(&left_view(&fx.frame), &mut detections)
        .unwrap();
    assert!(
        detections
            .iter()
            .all(|d| d.region == Region::TextChat && d.kind == NameKind::ChatSender)
    );
    let boxes = prepared(&fx.frame, &detections);
    let items: Vec<_> = fx.truth.in_region(Region::TextChat).collect();
    let describe = |i: &&blurrd_testkit::TruthItem| {
        format!(
            "{:?} {} {}",
            i.id,
            i.text,
            i.glyphs.bounds().unwrap_or(i.text)
        )
    };
    let leaks = item_coverage(&fx.truth, &boxes)
        .iter()
        .filter(|c| c.is_leak())
        .map(|c| {
            let item = items.iter().find(|i| i.id == c.id).unwrap();
            format!(
                "{} ({} of {} px)",
                describe(item),
                c.leaked_pixels,
                c.glyph_pixels
            )
        })
        .collect();
    let missed = items
        .iter()
        .filter(|i| {
            !detections
                .iter()
                .any(|d| i.text.intersection(&d.text).is_some())
        })
        .map(describe)
        .collect();
    let false_positives = detections
        .iter()
        .filter(|d| !items.iter().any(|i| i.text.intersection(&d.text).is_some()))
        .map(|d| format!("{}", d.text))
        .collect();
    Check {
        items: items.len(),
        over_blur: over_blur(&fx.truth, &boxes),
        safe: detector
            .lines()
            .iter()
            .any(|l| l.start == NameStart::Shortest || l.end == NameEnd::Safe),
        detections,
        leaks,
        missed,
        false_positives,
    }
}

fn check_frame(fx: &Fixture) -> Check {
    check_with(&mut ChatDetector::new(), fx)
}

#[test]
fn finds_the_sender_of_every_line() {
    let channels = [Channel::All, Channel::Team, Channel::Party];
    let mut names = 0;
    for seed in 0..30u64 {
        let mut rng = Rng::new(seed);
        let lines = 1 + (seed % 3) as u32;
        let messages = (0..lines)
            .map(|i| {
                let channel = channels[((seed + u64::from(i)) % 3) as usize];
                let style = SINGLE_WORD[((seed + u64::from(i)) % 5) as usize];
                message(&mut rng, channel, style, i)
            })
            .collect();
        let background = BACKGROUNDS[(seed % 2) as usize];
        let fx = chat_fixture(Language::English, messages, background, Palette::DEFAULT);
        let check = check_frame(&fx);
        let context = format!("seed {seed}, {lines} lines");
        check.assert_clean(&context);
        assert_eq!(check.items, lines as usize, "{context}");
        assert!(!check.safe, "{context}: {:?}", check.detections);
        assert!(
            check.over_blur.ratio() <= 0.35,
            "{context}: over-blur {:?}",
            check.over_blur
        );
        names += check.items;
    }
    assert!(names >= 55, "{names} names checked");
}

/// Every channel, system lines included.
const CHANNELS: [Channel; 5] = [
    Channel::All,
    Channel::Team,
    Channel::Party,
    Channel::Broadcast,
    Channel::System,
];

/// Over-blur summed over many frames, split by geometry.
#[derive(Debug, Default)]
struct OverBlurTotals {
    /// Frames whose lines all have both ends of their names found.
    tight: OverBlur,
    /// Frames with a line covered with Safe geometry.
    safe: OverBlur,
}

impl OverBlurTotals {
    fn add(&mut self, check: &Check) {
        let total = if check.safe {
            &mut self.safe
        } else {
            &mut self.tight
        };
        total.blurred += check.over_blur.blurred;
        total.outside += check.over_blur.outside;
    }

    /// Checks the gates of plan 11.3: over-blur at most 0.35 where names
    /// are found exactly, at most 0.6 with Safe geometry.
    fn assert_within_gates(&self, context: &str) {
        assert!(
            self.tight.ratio() <= 0.35 && self.safe.ratio() <= 0.6,
            "{context}: over-blur {self:?} (tight {:.3}, safe {:.3})",
            self.tight.ratio(),
            self.safe.ratio()
        );
    }
}

/// Checks chats of one to three random lines in every channel with every
/// name style, adds their over-blur to `totals` and returns the number of
/// names checked.
fn check_mixed_chats(
    language: Language,
    palette: Palette,
    seeds: std::ops::Range<u64>,
    totals: &mut OverBlurTotals,
) -> usize {
    let mut names = 0;
    for seed in seeds {
        let mut rng = Rng::new(seed);
        let lines = 1 + (seed % 3) as u32;
        let messages: Vec<_> = (0..lines)
            .map(|i| {
                let channel = CHANNELS[((seed + u64::from(i)) % 5) as usize];
                let style = NameStyle::LATIN_MIX[((3 * seed + u64::from(i)) % 7) as usize];
                message(&mut rng, channel, style, i)
            })
            .collect();
        let system = messages.iter().any(|m| m.channel == Channel::System);
        let background = BACKGROUNDS[(seed % 2) as usize];
        let fx = chat_fixture(language, messages, background, palette);
        let check = check_frame(&fx);
        let context = format!("{language:?}, {}, seed {seed}, {lines} lines", palette.name);
        check.assert_clean(&context);
        assert_eq!(check.items, lines as usize, "{context}");
        // System lines have no colon and a white name: always Safe.
        assert!(!system || check.safe, "{context}: {:?}", check.detections);
        totals.add(&check);
        names += check.items;
    }
    names
}

#[test]
fn finds_names_in_every_language_and_channel() {
    // German and Spanish prefixes have other widths and words; names have
    // every length, two words, digits and diacritics; backgrounds include
    // a bright sky that narrows the gaps between words.
    let mut names = 0;
    for language in Language::ALL {
        let mut totals = OverBlurTotals::default();
        names += check_mixed_chats(language, Palette::DEFAULT, 0..40, &mut totals);
        totals.assert_within_gates(&format!("{language:?}"));
    }
    assert!(names >= 220, "{names} names checked");
}

#[test]
fn finds_names_in_colour_blind_palettes() {
    let mut names = 0;
    for palette in &Palette::ALL[1..] {
        let mut totals = OverBlurTotals::default();
        names += check_mixed_chats(Language::English, *palette, 100..120, &mut totals);
        totals.assert_within_gates(palette.name);
    }
    assert!(names >= 110, "{names} names checked");
}

#[test]
fn skips_the_continuation_of_wrapped_messages() {
    for seed in 0..6u64 {
        let mut rng = Rng::new(40 + seed);
        let mut wrapped = message(&mut rng, Channel::All, NameStyle::Latin(9), 1);
        wrapped.text = "watch the flank and save the ult for the retake".to_owned();
        wrapped.wrap = true;
        let messages = vec![
            message(&mut rng, Channel::Team, NameStyle::Latin(6), 0),
            wrapped,
        ];
        let fx = chat_fixture(
            Language::English,
            messages,
            BACKGROUNDS[(seed % 2) as usize],
            Palette::DEFAULT,
        );
        let check = check_frame(&fx);
        check.assert_clean(&format!("seed {seed}"));
        assert_eq!(check.detections.len(), 2, "seed {seed}");
    }
}

#[test]
fn reports_the_message_after_the_name() {
    // The message is reported separately (blurred only when the user asks
    // for it), and the name box does not reach into it.
    for language in Language::ALL {
        let mut rng = Rng::new(7);
        let messages = vec![
            message(&mut rng, Channel::Team, NameStyle::Latin(8), 0),
            message(&mut rng, Channel::All, NameStyle::TwoWord, 1),
        ];
        let fx = chat_fixture(language, messages, Background::noise(4), Palette::DEFAULT);
        let mut detector = ChatDetector::new();
        check_with(&mut detector, &fx).assert_clean(&format!("{language:?}"));
        let bodies: Vec<_> = fx
            .truth
            .content
            .iter()
            .filter(|c| c.kind == ContentKind::ChatMessage)
            .collect();
        assert_eq!(detector.lines().len(), 2);
        for line in detector.lines() {
            assert_eq!(line.end, NameEnd::Colon, "{language:?}: {line:?}");
            let message = line.message.unwrap();
            assert!(message.x0 > line.name.x1, "{language:?}: {line:?}");
            let body = bodies
                .iter()
                .find(|b| b.rect.intersection(&message).is_some())
                .unwrap();
            assert!(
                message.x1 >= body.rect.x1 - 3,
                "{language:?}: {message} vs {}",
                body.rect
            );
        }
    }
}

#[test]
fn reports_nothing_without_the_chat_box() {
    let mut detector = ChatDetector::new();
    for background in BACKGROUNDS {
        let fx = Scene::new(Layout::reference(), background, Palette::DEFAULT, 1).render(0);
        let mut out = vec![];
        detector.detect(&left_view(&fx.frame), &mut out).unwrap();
        assert!(out.is_empty() && detector.bands().is_empty() && detector.lines().is_empty());
    }
}

#[test]
fn rejects_crops_of_another_size() {
    let data = vec![0u8; 3 * 100 * 100];
    let crop = RgbView::new(&data, 100, 100, 300).unwrap();
    assert_eq!(
        ChatDetector::new().detect(&crop, &mut vec![]),
        Err(DetectError::CropSize {
            width: 780,
            height: 490,
            actual_width: 100,
            actual_height: 100,
        })
    );
}
