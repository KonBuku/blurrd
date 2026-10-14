//! The text-chat detector on synthetic frames from blurrd-testkit: the
//! sender of every chat line is found and fully covered, the message is
//! not.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small frames: coordinates never wrap, a panic is a failure"
)]

mod support;

use blurrd_core::{NameKind, RectI, Region};
use blurrd_detect::chat::{
    BACKFILL_MAX, ChatDetector, ChatTracker, FADE_MAX, LEFT_PANEL_ROI, NameEnd, NameStart,
    POST_ROLL,
};
use blurrd_detect::{DetectError, Detection, RgbView};
use blurrd_testkit::hud::chat::{Channel, ChatInput, ChatMessage, ChatSpec, INPUT_Y, Language};
use blurrd_testkit::hud::killfeed::Team;
use blurrd_testkit::metrics::{OverBlur, item_coverage, over_blur};
use blurrd_testkit::{
    Background, ContentKind, Fixture, FontSet, Layout, NameStyle, Palette, RgbFrame, Rng, Scene,
    synth_name,
};

use support::{Filter, prepared};

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
    let mut totals = OverBlurTotals::default();
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
        let background = BACKGROUNDS[(seed / 3 % 3) as usize];
        let fx = chat_fixture(Language::English, messages, background, Palette::DEFAULT);
        let check = check_frame(&fx);
        let context = format!("seed {seed}, {lines} lines");
        check.assert_clean(&context);
        assert_eq!(check.items, lines as usize, "{context}");
        // On the bright sky the space after the prefix can close up and
        // faint names lose their colour: Safe geometry is allowed there.
        assert!(
            !check.safe || background == Background::white_sky(),
            "{context}: {:?}",
            check.detections
        );
        totals.add(&check);
        names += check.items;
    }
    totals.assert_within_gates("single-word names");
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
        let background = BACKGROUNDS[(seed / 3 % 3) as usize];
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
fn ignores_texture_at_the_box_edge() {
    // Bright background texture next to the left box edge, in the rows of
    // every line, is no prefix glyph.
    for seed in 0..6u64 {
        let mut rng = Rng::new(60 + seed);
        let messages = (0..3)
            .map(|i| message(&mut rng, Channel::All, SINGLE_WORD[(seed % 5) as usize], i))
            .collect();
        let mut fx = chat_fixture(
            Language::English,
            messages,
            BACKGROUNDS[(seed % 2) as usize],
            Palette::DEFAULT,
        );
        for top in [948, 977, 1006] {
            let speck = RectI {
                x0: 30,
                y0: top + 2,
                x1: 32,
                y1: top + 9,
            };
            fx.frame.fill_rect(speck, [235, 235, 235], 1.0);
        }
        let check = check_frame(&fx);
        check.assert_clean(&format!("seed {seed}"));
        assert_eq!(check.detections.len(), 3, "seed {seed}");
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

/// The tracked detections of a chat sequence, checked frame by frame.
#[derive(Debug, Default)]
struct SequenceCheck {
    /// Frames and names in them.
    frames: usize,
    names: usize,
    /// Name-frames with glyph pixels no box covers.
    leaks: Vec<String>,
    /// Boxes that overlap no name.
    false_positives: usize,
    /// Boxes that reach into the open chat's input line (the player's own
    /// typed text).
    on_input_line: usize,
    over_blur: OverBlur,
}

/// The dark weapon model, whose own edge lies on the right edge of the
/// chat box: the box darkens the frame inside, but the weapon outside is
/// darker still.
const WEAPON: RectI = RectI {
    x0: 470,
    y0: 760,
    x1: 600,
    y1: 1080,
};

/// Runs the detector and the tracker over every frame of a scene with
/// `spec`, with the left panel filtered by `filter` and a black object
/// drawn over `occluder` if given, and checks the tracked boxes against
/// each frame's truth.
fn check_sequence(
    spec: ChatSpec,
    background: Background,
    frames: u32,
    filter: Option<Filter>,
    occluder: Option<RectI>,
) -> SequenceCheck {
    let fonts = FontSet::latin();
    let scene = Scene::new(Layout::reference(), background, Palette::DEFAULT, frames)
        .with_chat(&fonts, spec);
    let mut detector = ChatDetector::new();
    let mut tracker = ChatTracker::new();
    let mut detections = vec![];
    let mut ready = vec![];
    let mut truths = std::collections::VecDeque::new();
    let mut check = SequenceCheck::default();
    let mut last_frame = None;
    let mut evaluate =
        |ready: &mut Vec<(u32, Vec<Detection>)>,
         truths: &mut std::collections::VecDeque<(u32, blurrd_testkit::FrameTruth)>,
         frame: &RgbFrame| {
            for (index, boxes) in ready.drain(..) {
                let (truth_index, truth) = truths.pop_front().unwrap();
                assert_eq!(index, truth_index);
                let prepared = prepared(frame, &boxes);
                let items: Vec<_> = truth.in_region(Region::TextChat).collect();
                check.frames += 1;
                check.names += items.len();
                for coverage in item_coverage(&truth, &prepared) {
                    if coverage.is_leak() {
                        let item = items.iter().find(|i| i.id == coverage.id).unwrap();
                        check.leaks.push(format!(
                            "frame {index} {:?} opacity {:.2} ({} of {} px)",
                            item.id, item.opacity, coverage.leaked_pixels, coverage.glyph_pixels
                        ));
                    }
                }
                check.false_positives += boxes
                    .iter()
                    .filter(|d| !items.iter().any(|i| i.text.intersection(&d.rect).is_some()))
                    .count();
                check.on_input_line += boxes.iter().filter(|d| d.rect.y1 > INPUT_Y.0).count();
                let blur = over_blur(&truth, &prepared);
                check.over_blur.blurred += blur.blurred;
                check.over_blur.outside += blur.outside;
            }
        };
    for index in 0..frames {
        let fx = scene.render(index);
        let mut frame = match filter {
            Some(filter) => filter.region(&fx.frame, LEFT_PANEL_ROI),
            None => fx.frame,
        };
        if let Some(rect) = occluder {
            frame.fill_rect(rect, [3, 3, 4], 1.0);
        }
        detector
            .detect(&left_view(&frame), &mut detections)
            .unwrap();
        tracker.push(index, &detector);
        tracker.drain_ready(&mut ready);
        truths.push_back((index, fx.truth));
        evaluate(&mut ready, &mut truths, &frame);
        last_frame = Some(frame);
    }
    tracker.finish(&mut ready);
    evaluate(&mut ready, &mut truths, &last_frame.unwrap());
    assert!(truths.is_empty(), "every frame comes out of the tracker");
    check
}

/// Runs `check` on every input on its own thread (each run renders and
/// tracks a whole sequence) and returns the results in input order.
fn in_parallel<T: Sync, R: Send>(inputs: &[T], check: impl Fn(&T) -> R + Sync) -> Vec<R> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .iter()
            .map(|input| scope.spawn(|| check(input)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    })
}

/// Frames of [`sequence`]: the last line fades out before its end.
const SEQUENCE_FRAMES: u32 = 260;

/// A chat sequence: single arrivals that push older lines up, two
/// messages in consecutive frames, a wrapped message and a system line,
/// with lines fading out in between.
fn sequence(rng: &mut Rng, language: Language) -> ChatSpec {
    let mut messages = vec![
        message(rng, Channel::All, NameStyle::Latin(8), 2),
        message(rng, Channel::Team, NameStyle::Latin(12), 30),
        message(rng, Channel::Party, NameStyle::Latin(5), 31),
        message(rng, Channel::All, NameStyle::TwoWord, 95),
        message(rng, Channel::Broadcast, NameStyle::Latin(10), 110),
        message(rng, Channel::System, NameStyle::Latin(9), 150),
        message(rng, Channel::All, NameStyle::Digits(6), 165),
    ];
    "watch the flank and save the ult for the retake".clone_into(&mut messages[3].text);
    messages[3].wrap = true;
    ChatSpec {
        life: 60,
        ..ChatSpec::new(language, messages)
    }
}

#[test]
fn tracker_covers_lines_through_pushes_and_fades() {
    // Every frame of every line is covered: the first frames after a line
    // pops in, the frames in which older lines move up, the whole fade-out
    // (text and box fade together), on dark, textured and bright
    // backgrounds and in every language.
    let mut over_blur = OverBlur::default();
    let mut names = 0;
    // Every language and every background, the bright sky twice.
    let runs = [(0, 0), (1, 1), (2, 2), (0, 2)];
    let checks = in_parallel(&runs, |&(k, b)| {
        let mut rng = Rng::new(500 + 3 * k as u64 + b as u64);
        check_sequence(
            sequence(&mut rng, Language::ALL[k]),
            BACKGROUNDS[b],
            SEQUENCE_FRAMES,
            None,
            None,
        )
    });
    for ((k, b), check) in runs.into_iter().zip(checks) {
        let context = format!("{:?}, background {b}", Language::ALL[k]);
        assert!(check.leaks.is_empty(), "{context}: {:?}", check.leaks);
        // Boxes outlive their line by at most the post-roll (seven lines),
        // and a line pops in at most two frames early.
        assert!(
            check.false_positives <= 7 * POST_ROLL as usize + 2,
            "{context}: {} false-positive boxes",
            check.false_positives
        );
        assert_eq!(check.frames, SEQUENCE_FRAMES as usize, "{context}");
        over_blur.blurred += check.over_blur.blurred;
        over_blur.outside += check.over_blur.outside;
        names += check.names;
    }
    // Safe geometry is part of the mix (system lines, bright backgrounds).
    assert!(
        over_blur.ratio() <= 0.6,
        "over-blur {over_blur:?} ({:.3})",
        over_blur.ratio()
    );
    assert!(names >= 4 * 7 * 70, "{names} name-frames checked");
}

#[test]
fn tracker_covers_lines_in_filtered_video() {
    // GPU-driver and editing filters change the box contrast and the name
    // colours: greyscale and vivid video, and lifted or crushed levels (a
    // lifted box looks faded by a fixed threshold, so its darkness is
    // learned per video). Hue shifts and gamma are covered by the
    // augmented recordings (plan 11.6).
    let filters = [
        Filter::Saturation(0.0),
        Filter::Saturation(1.8),
        Filter::Levels {
            brightness: 0.06,
            contrast: 0.85,
        },
        Filter::Levels {
            brightness: -0.12,
            contrast: 0.85,
        },
    ];
    let mut over_blur = OverBlur::default();
    let runs: Vec<(usize, Filter)> = filters.into_iter().enumerate().collect();
    let checks = in_parallel(&runs, |&(k, filter)| {
        let mut rng = Rng::new(700 + k as u64);
        check_sequence(
            sequence(&mut rng, Language::ALL[k % Language::ALL.len()]),
            BACKGROUNDS[k % 2],
            SEQUENCE_FRAMES,
            Some(filter),
            None,
        )
    });
    for ((k, filter), check) in runs.into_iter().zip(checks) {
        let language = Language::ALL[k % Language::ALL.len()];
        let context = format!("{filter:?}, {language:?}, background {}", k % 2);
        assert!(check.leaks.is_empty(), "{context}: {:?}", check.leaks);
        assert!(
            check.false_positives <= 7 * POST_ROLL as usize + 2,
            "{context}: {} false-positive boxes",
            check.false_positives
        );
        over_blur.blurred += check.over_blur.blurred;
        over_blur.outside += check.over_blur.outside;
    }
    assert!(
        over_blur.ratio() <= 0.6,
        "over-blur {over_blur:?} ({:.3})",
        over_blur.ratio()
    );
}

#[test]
fn tracker_returns_every_frame_once() {
    let mut detector = ChatDetector::new();
    let mut tracker = ChatTracker::new();
    let fx = Scene::new(Layout::reference(), Background::dusk(), Palette::DEFAULT, 1).render(0);
    let mut out = vec![];
    let mut ready = vec![];
    let frames = BACKFILL_MAX as u32 + 40;
    for frame in 0..frames {
        detector.detect(&left_view(&fx.frame), &mut out).unwrap();
        tracker.push(frame, &detector);
        tracker.drain_ready(&mut ready);
        assert_eq!(
            ready.len(),
            (frame + 1).saturating_sub(BACKFILL_MAX as u32) as usize
        );
    }
    tracker.finish(&mut ready);
    let returned: Vec<u32> = ready.iter().map(|(f, _)| *f).collect();
    assert_eq!(returned, (0..frames).collect::<Vec<_>>());
    assert!(ready.iter().all(|(_, d)| d.is_empty()));
}

#[test]
fn tracker_covers_lines_behind_an_edge_without_contrast() {
    // A dark weapon model whose edge lies on the right edge of the box
    // hides that edge: the left edge alone shows the box, from the frame a
    // line pops in.
    let runs = [0, 1, 2];
    let checks = in_parallel(&runs, |&b| {
        let mut rng = Rng::new(800 + b as u64);
        check_sequence(
            sequence(&mut rng, Language::ALL[b]),
            BACKGROUNDS[b],
            SEQUENCE_FRAMES,
            None,
            Some(WEAPON),
        )
    });
    for (b, check) in runs.into_iter().zip(checks) {
        assert!(check.leaks.is_empty(), "background {b}: {:?}", check.leaks);
        assert!(
            check.false_positives <= 7 * POST_ROLL as usize + 2,
            "background {b}: {} false-positive boxes",
            check.false_positives
        );
    }
}

/// Frames of [`fourth_line`] and [`open_chat`].
const LONG_FRAMES: u32 = 280;

/// A closed chat that shows four lines: the oldest line is pushed up above
/// the three-line grid while it fades (frames 66 to 80), and later four
/// fully visible lines stand on the grid.
fn fourth_line(rng: &mut Rng, language: Language) -> ChatSpec {
    let times = [2, 40, 50, 66, 150, 160, 170, 180];
    let messages = times
        .iter()
        .zip(0..)
        .map(|(&time, i)| {
            let style = NameStyle::LATIN_MIX[i % 7];
            message(rng, CHANNELS[i % 4], style, time)
        })
        .collect();
    ChatSpec {
        life: 60,
        shown: 4,
        ..ChatSpec::new(language, messages)
    }
}

#[test]
fn tracker_covers_a_fourth_line_above_the_grid() {
    let runs = [0, 1, 2];
    let checks = in_parallel(&runs, |&b| {
        let mut rng = Rng::new(900 + b as u64);
        check_sequence(
            fourth_line(&mut rng, Language::ALL[b]),
            BACKGROUNDS[b],
            LONG_FRAMES,
            None,
            None,
        )
    });
    for (b, check) in runs.into_iter().zip(checks) {
        assert!(check.leaks.is_empty(), "background {b}: {:?}", check.leaks);
        // Where gameplay texture keeps the edge ratio of a faded box part
        // below 0.97 by chance, a line can be held until FADE_MAX frames
        // after its band (two lines fade out on their own).
        assert!(
            check.false_positives <= 8 * POST_ROLL as usize + 2 * FADE_MAX as usize + 2,
            "background {b}: {} false-positive boxes",
            check.false_positives
        );
    }
}

/// A chat that is opened for typing at frame 100 and closed at frame 220:
/// the open chat shows the whole history, faded messages included, as far
/// as the panel reaches; two messages arrive while it is open (one of them
/// wrapped) and push the history up, out of the panel at its top.
fn open_chat(rng: &mut Rng, language: Language) -> ChatSpec {
    let mut messages: Vec<_> = (0..9)
        .map(|i| {
            let style = NameStyle::LATIN_MIX[i % 7];
            message(rng, CHANNELS[i % 5], style, 2 + 6 * i as u32)
        })
        .collect();
    messages.push(message(rng, Channel::All, NameStyle::Latin(10), 150));
    let mut wrapped = message(rng, Channel::Team, NameStyle::TwoWord, 175);
    "watch the flank and save the ult for the retake".clone_into(&mut wrapped.text);
    wrapped.wrap = true;
    messages.push(wrapped);
    ChatSpec {
        life: 60,
        input: Some(ChatInput {
            from: 100,
            to: 220,
            channel: Channel::Team,
            text: "rotate b".to_owned(),
        }),
        ..ChatSpec::new(language, messages)
    }
}

#[test]
fn finds_every_line_of_the_open_chat() {
    // The panel is full: eight lines on the grid, the oldest message
    // scrolled out at its top.
    for (k, language) in Language::ALL.into_iter().enumerate() {
        for (b, background) in BACKGROUNDS.into_iter().enumerate() {
            let mut rng = Rng::new(950 + 3 * k as u64 + b as u64);
            let fonts = FontSet::latin();
            let fx = Scene::new(Layout::reference(), background, Palette::DEFAULT, 200)
                .with_chat(&fonts, open_chat(&mut rng, language))
                .render(160);
            let check = check_frame(&fx);
            let context = format!("{language:?}, background {b}");
            check.assert_clean(&context);
            assert_eq!(check.items, 8, "{context}");
            assert!(
                check.detections.iter().all(|d| d.rect.y1 <= INPUT_Y.0),
                "{context}: the typed text is covered: {:?}",
                check.detections
            );
        }
    }
}

#[test]
fn finds_open_chat_lines_without_edge_contrast() {
    // Over the top lines of the open panel, an opaque overlay on the left
    // of the screen (a spectator's player card) hides the left box edge, a
    // black scene leaves both edges without a step, or a filter that lifts
    // the shadows of a black scene leaves a flat grey on both sides of the
    // edges of the whole panel (the scrollbar thumb stays white).
    let rect = |x0, y0, x1, y1| RectI { x0, y0, x1, y1 };
    let cases: [(&str, Vec<RectI>, [u8; 3]); 3] = [
        ("overlay", vec![rect(0, 795, 34, 880)], [120, 110, 100]),
        (
            "black",
            vec![rect(0, 795, 26, 880), rect(457, 795, 500, 880)],
            [1, 1, 1],
        ),
        (
            "flat",
            vec![
                rect(18, 787, 34, 1028),
                rect(446, 787, 458, 1028),
                rect(458, 787, 470, 945),
            ],
            [33, 33, 33],
        ),
    ];
    for (b, background) in BACKGROUNDS.into_iter().enumerate() {
        for (name, rects, colour) in &cases {
            let mut rng = Rng::new(970 + b as u64);
            let fonts = FontSet::latin();
            let mut fx = Scene::new(Layout::reference(), background, Palette::DEFAULT, 200)
                .with_chat(&fonts, open_chat(&mut rng, Language::English))
                .render(160);
            for rect in rects {
                fx.frame.fill_rect(*rect, *colour, 1.0);
            }
            let check = check_frame(&fx);
            let context = format!("background {b}, {name}");
            check.assert_clean(&context);
            assert_eq!(check.detections.len(), 8, "{context}");
        }
    }
}

#[test]
fn finds_closed_chat_lines_without_box_edges() {
    // A filter that crushes a dark scene next to the box to black or
    // lifts its black to a flat grey leaves no step at either box edge:
    // the lines whose name ends at its colon or its colour are still
    // found. (Over a bright scene the box edges stay visible.)
    let rect = |x0, y0, x1, y1| RectI { x0, y0, x1, y1 };
    let channels = [Channel::All, Channel::Team, Channel::Party];
    for seed in 0..6u64 {
        let mut rng = Rng::new(1100 + seed);
        let messages = (0..3)
            .map(|i| {
                message(
                    &mut rng,
                    channels[(seed as usize + i) % 3],
                    SINGLE_WORD[i],
                    i as u32,
                )
            })
            .collect();
        let mut fx = chat_fixture(
            Language::English,
            messages,
            BACKGROUNDS[(seed % 2) as usize],
            Palette::DEFAULT,
        );
        for colour in [[0, 0, 0], [33, 33, 33]] {
            for r in [rect(14, 925, 34, 1040), rect(461, 925, 481, 1040)] {
                fx.frame.fill_rect(r, colour, 1.0);
            }
            let mut detector = ChatDetector::new();
            let check = check_with(&mut detector, &fx);
            let context = format!("seed {seed}, edge colour {colour:?}");
            assert!(
                detector.bands().is_empty(),
                "{context}: {:?}",
                detector.bands()
            );
            check.assert_clean(&context);
            assert_eq!(check.detections.len(), 3, "{context}");
        }
    }
}

#[test]
fn finds_bracketed_lines_without_colon_or_box_edges() {
    // Broadcast and system lines have no colon, and their names do not end
    // at a colour: without box edges, the prefix in parentheses alone shows
    // that they are chat lines (measured on real recordings under
    // brightness -0.2: broadcast names lost for up to 60 frames).
    let rect = |x0, y0, x1, y1| RectI { x0, y0, x1, y1 };
    let channels = [Channel::Broadcast, Channel::System, Channel::Purchase];
    for seed in 0..4u64 {
        let mut rng = Rng::new(1200 + seed);
        let messages = (0..3)
            .map(|i| {
                message(
                    &mut rng,
                    channels[(seed as usize + i) % 3],
                    SINGLE_WORD[i + 1],
                    i as u32,
                )
            })
            .collect();
        let mut fx = chat_fixture(
            Language::English,
            messages,
            BACKGROUNDS[(seed % 2) as usize],
            Palette::DEFAULT,
        );
        for r in [rect(14, 925, 34, 1040), rect(461, 925, 481, 1040)] {
            fx.frame.fill_rect(r, [0, 0, 0], 1.0);
        }
        let mut detector = ChatDetector::new();
        let check = check_with(&mut detector, &fx);
        let context = format!("seed {seed}");
        assert!(detector.bands().is_empty(), "{context}");
        check.assert_clean(&context);
        assert!(check.detections.len() >= 3, "{context}");
    }
}

#[test]
fn finds_open_chat_names_too_faint_to_see_in_greyscale() {
    // On a bright background in greyscale video, a red name and its prefix
    // are darker than the panel around them and leave no bright strokes:
    // the line's first glyph is the colon.
    for (b, background) in BACKGROUNDS.into_iter().enumerate() {
        let mut rng = Rng::new(990 + b as u64);
        let fonts = FontSet::latin();
        let fx = Scene::new(Layout::reference(), background, Palette::DEFAULT, 200)
            .with_chat(&fonts, open_chat(&mut rng, Language::English))
            .render(160);
        let fx = Fixture {
            frame: Filter::Saturation(0.0).region(&fx.frame, LEFT_PANEL_ROI),
            ..fx
        };
        let check = check_frame(&fx);
        let context = format!("background {b}");
        check.assert_clean(&context);
        assert_eq!(check.detections.len(), 8, "{context}");
    }
}

#[test]
fn tracker_covers_the_open_chat() {
    // Every history line is covered from the frame the chat opens until it
    // closes, through the pushes of new messages; the typed text is not.
    let runs = [(0, 0), (1, 1), (2, 2)];
    let checks = in_parallel(&runs, |&(k, b)| {
        let mut rng = Rng::new(980 + 3 * k as u64 + b as u64);
        check_sequence(
            open_chat(&mut rng, Language::ALL[k]),
            BACKGROUNDS[b],
            LONG_FRAMES,
            None,
            None,
        )
    });
    for ((k, b), check) in runs.into_iter().zip(checks) {
        let context = format!("{:?}, background {b}", Language::ALL[k]);
        assert!(check.leaks.is_empty(), "{context}: {:?}", check.leaks);
        assert_eq!(check.on_input_line, 0, "{context}");
        // Lines that leave the panel at its top, or vanish when the chat
        // closes, stay covered for the post-roll.
        assert!(
            check.false_positives <= 12 * POST_ROLL as usize + 2,
            "{context}: {} false-positive boxes",
            check.false_positives
        );
        assert!(
            check.names >= 8 * 100,
            "{context}: {} name-frames",
            check.names
        );
    }
}

/// A purchase broadcast: `buyer` bought a weapon for `recipient`, both
/// names in white, the recipient on a continuation line if `wrap`.
fn purchase(
    rng: &mut Rng,
    buyer: NameStyle,
    recipient: NameStyle,
    wrap: bool,
    time: u32,
) -> ChatMessage {
    ChatMessage {
        name: synth_name(rng, buyer),
        text: synth_name(rng, recipient),
        role: None,
        wrap,
        ..message(rng, Channel::Purchase, buyer, time)
    }
}

#[test]
fn covers_both_names_of_a_purchase_broadcast() {
    // "(Broadcast) Buyer bought the Vandal for Recipient!": no colon and
    // white names, so the line is covered to its end (Safe geometry),
    // which covers the second name wherever the language puts it, on a
    // continuation line too.
    let mut totals = OverBlurTotals::default();
    let mut names = 0;
    for (k, language) in Language::ALL.into_iter().enumerate() {
        for (b, background) in BACKGROUNDS.into_iter().enumerate() {
            for (p, palette) in [Palette::DEFAULT, Palette::ALL[1]].into_iter().enumerate() {
                let seed = (9 * k + 3 * b + p) as u64;
                let mut rng = Rng::new(1100 + seed);
                let buyer = NameStyle::LATIN_MIX[(seed % 7) as usize];
                let recipient = NameStyle::LATIN_MIX[((seed + 3) % 7) as usize];
                let messages = vec![
                    message(&mut rng, Channel::Team, NameStyle::Latin(7), 0),
                    purchase(&mut rng, buyer, recipient, seed.is_multiple_of(3), 1),
                ];
                let fx = chat_fixture(language, messages, background, palette);
                let check = check_frame(&fx);
                let context = format!("{language:?}, background {b}, {}", palette.name);
                check.assert_clean(&context);
                assert_eq!(check.items, 3, "{context}");
                assert!(check.safe, "{context}: {:?}", check.detections);
                totals.add(&check);
                names += check.items;
            }
        }
    }
    totals.assert_within_gates("purchase broadcasts");
    assert_eq!(names, 3 * 3 * 2 * 3);
}

/// Frames of [`purchases`].
const PURCHASE_FRAMES: u32 = 200;

/// A chat sequence with purchase broadcasts, one of them wrapped, pushed
/// up by later messages and fading out.
fn purchases(rng: &mut Rng, language: Language) -> ChatSpec {
    let messages = vec![
        message(rng, Channel::All, NameStyle::Latin(8), 2),
        purchase(rng, NameStyle::Latin(12), NameStyle::Latin(16), false, 30),
        purchase(rng, NameStyle::TwoWord, NameStyle::Latin(5), true, 70),
        message(rng, Channel::Team, NameStyle::Latin(6), 100),
    ];
    ChatSpec {
        life: 60,
        ..ChatSpec::new(language, messages)
    }
}

#[test]
fn tracker_covers_both_names_of_a_purchase_broadcast() {
    let runs = [
        (0, 0, None),
        (1, 1, None),
        (2, 2, None),
        (0, 2, Some(Filter::Saturation(0.0))),
        (
            1,
            0,
            Some(Filter::Levels {
                brightness: -0.12,
                contrast: 0.85,
            }),
        ),
    ];
    let checks = in_parallel(&runs, |&(k, b, filter)| {
        let mut rng = Rng::new(1200 + 3 * k as u64 + b as u64);
        check_sequence(
            purchases(&mut rng, Language::ALL[k]),
            BACKGROUNDS[b],
            PURCHASE_FRAMES,
            filter,
            None,
        )
    });
    for ((k, b, filter), check) in runs.into_iter().zip(checks) {
        let context = format!("{:?}, background {b}, {filter:?}", Language::ALL[k]);
        assert!(check.leaks.is_empty(), "{context}: {:?}", check.leaks);
        // Boxes outlive their line by at most the post-roll (four lines),
        // and a line pops in at most two frames early. The wrapped
        // broadcast puts the oldest lines above the grid of three, where
        // gameplay texture can keep a faded box part below 0.97 and hold
        // a line until FADE_MAX frames after its band (two lines fade out
        // there on their own), as in the fourth-line test. With crushed
        // levels, a push above the grid can be taken one step too far,
        // which keeps a box one step above the line until it fades out.
        assert!(
            check.false_positives <= 4 * POST_ROLL as usize + 3 * FADE_MAX as usize + 2,
            "{context}: {} false-positive boxes",
            check.false_positives
        );
        assert!(
            check.names >= 6 * 60,
            "{context}: {} name-frames",
            check.names
        );
    }
}
