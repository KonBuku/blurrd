//! Pass 0 on synthetic HUD frames: the dictionary built from the protected
//! zones contains the names of every region (the OCR positive control of
//! plan 11.3, at least 90 % per region), at every resolution and layout of
//! the fixture matrix, with colour-blind palettes and with GPU colour
//! filters.

#![allow(
    clippy::cast_possible_wrap,
    clippy::unwrap_used,
    reason = "test helpers on small frames: coordinates never wrap, a panic is a failure"
)]

mod support;

use std::collections::BTreeMap;

use blurrd_core::{FrameRange, HudGeometry};
use blurrd_engine::DictionaryBuilder;
use blurrd_ocr::{Ep, OcrEngine, OcrOptions, RgbView};
use blurrd_testkit::hud::banner::BannerSpec;
use blurrd_testkit::hud::chat::{Channel, ChatMessage, ChatSpec, Language};
use blurrd_testkit::hud::killfeed::{Kill, KillFeedSpec, Team, Weapon};
use blurrd_testkit::hud::voice::{LOCATIONS, Speaker, VoiceSpec};
use blurrd_testkit::{Background, FontSet, Layout, NameStyle, Palette, Rng, Scene, synth_name};
use support::Filter;

/// The names shown in one synthetic frame, per region.
#[derive(Debug, Default)]
struct Shown {
    kill_feed: Vec<String>,
    banner: Vec<String>,
    voice: Vec<String>,
    chat: Vec<String>,
}

/// One synthetic recording: its layout, colours, background and seed.
#[derive(Debug, Clone)]
struct Case {
    /// The case group the rates are checked for (layout, palette, filter
    /// or background), the same for every seed.
    group: String,
    layout: Layout,
    palette: Palette,
    filter: Option<Filter>,
    background: Option<Background>,
    seed: u64,
}

impl Case {
    /// The reference layout with the default colours.
    fn reference(seed: u64) -> Self {
        Self {
            group: "reference".to_owned(),
            layout: Layout::reference(),
            palette: Palette::DEFAULT,
            filter: None,
            background: None,
            seed,
        }
    }

    /// The HUD geometry of the layout (what calibration finds).
    fn geometry(&self) -> HudGeometry {
        let l = &self.layout;
        HudGeometry {
            sx: l.sx,
            sy: l.sy,
            anchor_left: l.left,
            anchor_right: l.right,
            anchor_center: f32::midpoint(l.left, l.right),
            oy: l.top,
            layout: l.kind,
            ..HudGeometry::default()
        }
    }
}

/// The robustness matrix, `seeds` cases per group, each with its own
/// names: every layout of the fixture matrix (720p to 4K, native 4:3,
/// pillarbox, 16:10, stretched 4:3), the colour-blind palettes, GPU colour
/// filters, filters combined with layouts, and greyscale over a background
/// as bright as the coloured names.
fn matrix(seeds: u64) -> Vec<Case> {
    let mut groups: Vec<Case> = Vec::new();
    for layout in Layout::matrix() {
        groups.push(Case {
            group: format!("{}x{} {:?}", layout.width, layout.height, layout.kind),
            layout,
            ..Case::reference(0)
        });
    }
    for palette in &Palette::ALL[1..] {
        groups.push(Case {
            group: format!("palette {}", palette.name),
            palette: *palette,
            ..Case::reference(0)
        });
    }
    let filters = [
        Filter::Hue(120.0),
        Filter::Saturation(0.0),
        Filter::Saturation(1.8),
        Filter::Vibrance(1.0),
        Filter::Gamma(1.4),
        Filter::Levels {
            brightness: -0.12,
            contrast: 0.85,
        },
    ];
    for filter in filters {
        groups.push(Case {
            group: format!("{filter:?}"),
            filter: Some(filter),
            ..Case::reference(0)
        });
    }
    let combos = [
        (Layout::stretched_4x3(1920, 1080), Filter::Hue(120.0)),
        (Layout::wide(1280, 720), Filter::Saturation(0.0)),
        (
            Layout::pillarbox(1920, 1080),
            Filter::Levels {
                brightness: -0.12,
                contrast: 0.85,
            },
        ),
        (Layout::wide(3840, 2160), Filter::Gamma(1.4)),
    ];
    for (layout, filter) in combos {
        groups.push(Case {
            group: format!(
                "{}x{} {:?} + {filter:?}",
                layout.width, layout.height, layout.kind
            ),
            layout,
            filter: Some(filter),
            ..Case::reference(0)
        });
    }
    // Greyscale over a flat background of about the luma of the team
    // colours: coloured names keep only a small luma contrast.
    groups.push(Case {
        group: "Saturation(0.0) on a name-luma background".to_owned(),
        filter: Some(Filter::Saturation(0.0)),
        background: Some(Background::Flat([140, 140, 140])),
        ..Case::reference(0)
    });
    let mut cases = Vec::new();
    for (g, group) in groups.into_iter().enumerate() {
        for s in 0..seeds {
            cases.push(Case {
                seed: 10 + 16 * g as u64 + s,
                ..group.clone()
            });
        }
    }
    cases
}

/// A scene with five kill-feed rows, a banner, three voice speakers and
/// four chat lines, all settled by frame 120.
fn scene(fonts: &FontSet, case: &Case) -> (Scene, Shown) {
    let seed = case.seed;
    let mut rng = Rng::new(seed);
    let name = |rng: &mut Rng| {
        let style = *rng.pick(&NameStyle::LATIN_MIX);
        synth_name(rng, style)
    };
    let mut shown = Shown::default();
    let kills: Vec<Kill> = (0..5)
        .map(|i| {
            let (killer, victim) = (name(&mut rng), name(&mut rng));
            shown.kill_feed.push(killer.clone());
            shown.kill_feed.push(victim.clone());
            Kill {
                killer,
                victim,
                killer_team: Team::Ally,
                victim_team: Team::Enemy,
                weapon: Weapon::Rifle,
                badge: None,
                own: false,
                headshot: i % 2 == 0,
                birth: i * 4,
            }
        })
        .collect();
    let banner = name(&mut rng);
    shown.banner.push(banner.clone());
    let speakers: Vec<Speaker> = (0..3u8)
        .map(|slot| {
            let speaker = name(&mut rng);
            shown.voice.push(speaker.clone());
            Speaker {
                name: speaker,
                location: LOCATIONS[slot as usize].to_owned(),
                slot,
                shown: vec![FrameRange { f0: 0, f1: 10_000 }],
            }
        })
        .collect();
    // The chat shows its three newest lines; rotate through the channels.
    let channels = [
        Channel::All,
        Channel::Team,
        Channel::Party,
        Channel::System,
        Channel::Broadcast,
    ];
    let messages: Vec<ChatMessage> = (0..3)
        .map(|i| {
            let channel = channels[(seed as usize + i) % channels.len()];
            let sender = name(&mut rng);
            shown.chat.push(sender.clone());
            ChatMessage {
                channel,
                name: sender,
                team: if i % 2 == 0 { Team::Enemy } else { Team::Ally },
                text: "rotate mid now".to_owned(),
                role: (channel == Channel::Broadcast).then(|| "Caster".to_owned()),
                wrap: false,
                time: i as u32 * 3,
            }
        })
        .collect();
    let background = case.background.unwrap_or(Background::noise(seed));
    let scene = Scene::new(case.layout, background, case.palette, 400)
        .with_kill_feed(fonts, KillFeedSpec::new(kills))
        .with_banner(fonts, BannerSpec::new("ACE", &banner, 40))
        .with_voice(fonts, VoiceSpec { speakers })
        .with_chat(fonts, ChatSpec::new(Language::English, messages));
    (scene, shown)
}

/// Names read and names shown, per case group and region.
type Rates = BTreeMap<(String, &'static str), (usize, usize)>;

/// Builds the dictionary over a few frames of every case and counts per
/// case group and region the names read and the names shown.
fn positive_control(ep: Ep, cases: &[Case]) -> Option<Rates> {
    let (runtime, models) = support::ocr()?;
    let fonts = FontSet::latin();
    let mut rates = Rates::new();
    // Zone sizes depend only on the reference geometry, so the sessions
    // serve every layout.
    let options = OcrOptions {
        ep,
        zones: DictionaryBuilder::new(&HudGeometry::default(), 1920, 1080).zone_specs(),
        ..OcrOptions::default()
    };
    let mut engine = OcrEngine::new(&runtime, &models, &options).unwrap();
    for case in cases {
        let (width, height) = (case.layout.width, case.layout.height);
        let mut builder = DictionaryBuilder::new(&case.geometry(), width, height);
        let (scene, shown) = scene(&fonts, case);
        for index in [120, 126, 132] {
            let mut data = scene.render(index).frame.data().to_vec();
            if let Some(filter) = case.filter {
                filter.apply(&mut data);
            }
            let view = RgbView::new(width, height, &data).unwrap();
            builder.push_frame(&mut engine, view).unwrap();
        }
        let stats = builder.stats();
        // The frames are identical apart from the banner hold, so the
        // unchanged zones are read once.
        assert!(stats.zone_crops_read < stats.zone_crops, "{stats:?}");
        let dict = builder.dictionary();
        for (region, names) in [
            ("kill feed", &shown.kill_feed),
            ("banner", &shown.banner),
            ("voice", &shown.voice),
            ("chat", &shown.chat),
        ] {
            let entry = rates.entry((case.group.clone(), region)).or_default();
            entry.0 += names.iter().filter(|n| support::was_read(dict, n)).count();
            entry.1 += names.len();
        }
    }
    // Every session kept the shape it was built for; stalls stay rare.
    support::assert_fixed_shapes(&engine, &options.zones);
    support::assert_stalls_tolerable(&engine);
    Some(rates)
}

/// The smallest sample of one case group and region whose rate is
/// checked: below it a single miss moves the rate by more than 3 points,
/// so smaller samples are only reported (they still count in the totals).
const MIN_GROUP_SAMPLE: usize = 30;

/// Prints the rates per region over all cases and per case group, and
/// checks the positive control: at least 90 % of the names of every region
/// over all cases, and in every case group that shows a region at least
/// [`MIN_GROUP_SAMPLE`] times.
fn assert_rates(rates: &Rates) {
    let mut totals: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    for ((_, region), (read, shown)) in rates {
        let total = totals.entry(region).or_default();
        total.0 += read;
        total.1 += shown;
    }
    let percent = |read: usize, shown: usize| 100.0 * read as f32 / shown.max(1) as f32;
    let mut failed = Vec::new();
    for (region, (read, shown)) in &totals {
        eprintln!(
            "{region}: {read}/{shown} names read ({:.1} %)",
            percent(*read, *shown)
        );
        if (*read as f32) < 0.9 * *shown as f32 {
            failed.push(format!("{region}: {read}/{shown}"));
        }
    }
    for ((group, region), (read, shown)) in rates {
        let below = (*read as f32) < 0.9 * *shown as f32;
        let checked = *shown >= MIN_GROUP_SAMPLE;
        let note = match (below, checked) {
            (false, _) => "",
            (true, false) => "  (sample too small to check)",
            (true, true) => "  below 90 %",
        };
        eprintln!(
            "  {group} / {region}: {read}/{shown} ({:.1} %){note}",
            percent(*read, *shown)
        );
        if below && checked {
            failed.push(format!("{group} / {region}: {read}/{shown}"));
        }
    }
    assert!(failed.is_empty(), "below 90 %: {failed:?}");
}

#[test]
fn cpu_dictionary_holds_the_names_of_every_region() {
    let cases: Vec<Case> = (0..3).map(Case::reference).collect();
    if let Some(rates) = positive_control(Ep::Cpu, &cases) {
        assert_rates(&rates);
    }
}

#[test]
fn cpu_names_are_read_at_every_layout_palette_and_filter() {
    if let Some(rates) = positive_control(Ep::Cpu, &matrix(1)) {
        assert_rates(&rates);
    }
}

#[test]
#[ignore = "needs a DirectX 12 GPU"]
fn gpu_names_are_read_at_every_layout_palette_and_filter() {
    let mut cases: Vec<Case> = (0..12).map(Case::reference).collect();
    cases.extend(matrix(10));
    if let Some(rates) = positive_control(Ep::DirectMl, &cases) {
        assert_rates(&rates);
    }
}
