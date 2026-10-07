//! Property tests for tracks, plan lookups and override binding.

use blurrd_core::{
    BindStatus, BlurBox, BlurPlan, Confidence, NameKind, Override, RectI, Region, RegionSet,
    Segment, Track, TrackId, TrackSource,
};
use proptest::prelude::*;
use uuid::Uuid;

const WIDTH: u32 = 256;
const HEIGHT: u32 = 128;

/// Rectangles in a small frame so that overrides often overlap tracks.
fn rect() -> impl Strategy<Value = RectI> {
    (0..200_i32, 0..100_i32, 4..56_i32, 4..28_i32).prop_map(|(x, y, w, h)| RectI {
        x0: x,
        y0: y,
        x1: x + w,
        y1: y + h,
    })
}

fn confidence() -> impl Strategy<Value = Confidence> {
    prop_oneof![
        Just(Confidence::Tracked),
        Just(Confidence::WeakHold),
        Just(Confidence::GapFill),
        Just(Confidence::PreRoll),
        Just(Confidence::PostRoll),
        Just(Confidence::ColourPrior),
        Just(Confidence::OcrFix),
    ]
}

fn region() -> impl Strategy<Value = Region> {
    prop::sample::select(Region::DETECTED.to_vec())
}

/// A valid track: segments separated by gaps of at least one frame.
fn track(id: u32, frame_count: u32) -> impl Strategy<Value = Track> {
    (
        region(),
        prop::collection::vec((0..20_u32, 0..30_u32, rect(), confidence()), 1..6),
    )
        .prop_map(move |(region, parts)| {
            let mut segments = Vec::new();
            let mut next = 0_u32;
            for (gap, len, rect, conf) in parts {
                let f0 = next + gap;
                let f1 = f0 + len;
                if f1 >= frame_count {
                    break;
                }
                let offset = (rect.x0 - 100, rect.y0 - 50);
                segments.push(Segment {
                    f0,
                    f1,
                    rect,
                    conf,
                    text_offset: (offset.0 as i16, offset.1 as i16),
                });
                next = f1 + 1;
            }
            if segments.is_empty() {
                segments.push(Segment {
                    f0: 0,
                    f1: 0,
                    rect: RectI {
                        x0: 0,
                        y0: 0,
                        x1: 8,
                        y1: 8,
                    },
                    conf: Confidence::Tracked,
                    text_offset: (0, 0),
                });
            }
            Track {
                id: TrackId(id),
                region,
                kind: NameKind::Killer,
                source: TrackSource::Detector,
                segments,
            }
        })
}

/// An override, with its anchor either near an existing segment or random.
fn edit(tracks: Vec<Track>, frame_count: u32, n: u128) -> impl Strategy<Value = Override> {
    let pick = (0..tracks.len(), 0..6_usize, -6..6_i32, -6..6_i32);
    (
        0..3_u8,
        pick,
        rect(),
        rect(),
        region(),
        0..frame_count,
        0..60_u32,
        any::<bool>(),
    )
        .prop_map(
            move |(op, (t, s, dx, dy), random_anchor, new_rect, random_region, f0, len, near)| {
                let uid = Uuid::from_u128(n);
                let f1 = (f0 + len).min(frame_count - 1);
                let track = &tracks[t];
                let segment = &track.segments[s % track.segments.len()];
                let (region, anchor) = if near {
                    let r = segment.rect;
                    let (w, h) = (r.width().cast_signed(), r.height().cast_signed());
                    let x0 = (r.x0 + dx).clamp(0, WIDTH.cast_signed() - w);
                    let y0 = (r.y0 + dy).clamp(0, HEIGHT.cast_signed() - h);
                    (
                        track.region,
                        RectI {
                            x0,
                            y0,
                            x1: x0 + w,
                            y1: y0 + h,
                        },
                    )
                } else {
                    (random_region, random_anchor)
                };
                match op {
                    0 => Override::Add {
                        uid,
                        region: Region::Manual,
                        f0,
                        f1,
                        rect: new_rect,
                    },
                    1 => Override::Suppress {
                        uid,
                        region,
                        f0,
                        f1,
                        anchor,
                    },
                    _ => Override::Adjust {
                        uid,
                        region,
                        f0,
                        f1,
                        anchor,
                        rect: new_rect,
                    },
                }
            },
        )
}

fn plan() -> impl Strategy<Value = BlurPlan> {
    (20..400_u32, 1..10_u32)
        .prop_flat_map(|(frame_count, n)| {
            let tracks: Vec<_> = (0..n).map(|id| track(id, frame_count)).collect();
            (Just(frame_count), tracks)
        })
        .prop_flat_map(|(frame_count, tracks)| {
            let edits: Vec<_> = (0..8_u128)
                .map(|n| edit(tracks.clone(), frame_count, n + 1))
                .collect();
            (Just(frame_count), Just(tracks), edits)
        })
        .prop_map(|(frame_count, tracks, overrides)| BlurPlan {
            frame_count,
            tracks,
            overrides,
        })
}

fn enabled() -> impl Strategy<Value = RegionSet> {
    prop::sample::subsequence(Region::DETECTED.to_vec(), 0..=4)
        .prop_map(|regions| regions.into_iter().collect())
}

/// The plan without overrides of the given kind.
fn without(plan: &BlurPlan, keep: impl Fn(&Override) -> bool) -> BlurPlan {
    BlurPlan {
        overrides: plan.overrides.iter().copied().filter(keep).collect(),
        ..plan.clone()
    }
}

fn is_add(edit: &Override) -> bool {
    matches!(edit, Override::Add { .. })
}

proptest! {
    #[test]
    fn generated_plans_are_valid(plan in plan()) {
        prop_assert_eq!(plan.validate(WIDTH, HEIGHT), Ok(()));
    }

    #[test]
    fn segment_lookup_matches_a_linear_scan(plan in plan()) {
        for track in &plan.tracks {
            for frame in 0..plan.frame_count {
                let expected = track.segments.iter().find(|s| s.contains(frame));
                prop_assert_eq!(track.segment_at(frame), expected);
            }
        }
    }

    #[test]
    fn frame_table_matches_per_frame_lookup(plan in plan(), enabled in enabled()) {
        let table = plan.frame_table(&enabled);
        prop_assert_eq!(table.frame_count(), plan.frame_count);
        let mut out = Vec::new();
        let mut total = 0;
        for frame in 0..plan.frame_count {
            plan.boxes_at(frame, &enabled, &mut out);
            prop_assert_eq!(table.boxes(frame), out.as_slice());
            prop_assert!(out.iter().all(|b| b.rect.is_inside(WIDTH, HEIGHT)));
            total += out.len();
        }
        prop_assert_eq!(table.total_boxes(), total);
        prop_assert!(table.boxes(plan.frame_count).is_empty());
    }

    #[test]
    fn plain_tracks_yield_one_box_per_segment_frame(plan in plan(), enabled in enabled()) {
        let plain = without(&plan, |_| false);
        let expected: u32 = plain
            .tracks
            .iter()
            .filter(|t| enabled.contains(t.region))
            .flat_map(|t| &t.segments)
            .map(|s| s.f1 - s.f0 + 1)
            .sum();
        prop_assert_eq!(plain.frame_table(&enabled).total_boxes(), expected as usize);
    }

    #[test]
    fn unbound_overrides_have_no_effect(plan in plan()) {
        let bindings = plan.bindings();
        prop_assert_eq!(bindings.len(), plan.overrides.len());
        let bound: Vec<Uuid> = bindings
            .iter()
            .filter(|b| b.status == BindStatus::Bound)
            .map(|b| b.uid)
            .collect();
        prop_assert!(plan.overrides.iter().filter(|e| is_add(e)).all(|e| bound.contains(&e.uid())));
        let only_bound = without(&plan, |e| bound.contains(&e.uid()));
        let all = RegionSet::all();
        prop_assert_eq!(plan.frame_table(&all), only_bound.frame_table(&all));
    }

    /// Suppress and Adjust never retarget: a box disappears only where a
    /// bound Suppress of its region matches it by IoU >= 0.7, and changes
    /// only where a bound Adjust matches it by IoU >= 0.5.
    #[test]
    fn overrides_never_retarget(plan in plan()) {
        let all = RegionSet::all();
        let base = without(&plan, |_| false).frame_table(&all);
        let edited = plan.frame_table(&all);
        let bindings = plan.bindings();
        let active = |edit: &Override| {
            bindings.iter().any(|b| b.uid == edit.uid() && b.status == BindStatus::Bound)
        };
        for frame in 0..plan.frame_count {
            let before = base.boxes(frame);
            let after = edited.boxes(frame);
            let added = plan.overrides.iter().filter(|e| is_add(e) && e.covers(frame)).count();
            prop_assert!(after.len() >= added);
            let after_tracks = &after[..after.len() - added];
            // `after_tracks` is `before` with some boxes removed and some
            // rectangles replaced; walk both in order.
            let mut rest = after_tracks.iter().peekable();
            for old in before {
                let kept = rest.peek().is_some_and(|new| same_track_box(old, new));
                if kept {
                    let new = rest.next().unwrap_or(old);
                    if new.rect != old.rect {
                        let justified = plan.overrides.iter().any(|e| matches!(*e,
                            Override::Adjust { region, anchor, rect, .. }
                                if region == old.region && rect == new.rect && e.covers(frame)
                                    && old.rect.iou(&anchor) >= 0.5 && active(e)));
                        prop_assert!(justified, "unjustified change at frame {}", frame);
                    }
                } else {
                    let justified = plan.overrides.iter().any(|e| matches!(*e,
                        Override::Suppress { region, anchor, .. }
                            if region == old.region && e.covers(frame)
                                && old.rect.iou(&anchor) >= 0.7 && active(e)));
                    prop_assert!(justified, "unjustified removal at frame {}", frame);
                }
            }
            prop_assert!(rest.next().is_none(), "unexpected extra box at frame {}", frame);
        }
    }
}

/// Whether two boxes come from the same track box (the rectangle may have
/// been adjusted).
fn same_track_box(old: &BlurBox, new: &BlurBox) -> bool {
    old.seed == new.seed
        && old.region == new.region
        && old.grid_origin == new.grid_origin
        && old.block_scale.to_bits() == new.block_scale.to_bits()
}
