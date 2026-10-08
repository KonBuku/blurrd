//! The committed media manifest and, locally, the recordings and goldens
//! in `BLURRD_TEST_MEDIA`.

use blurrd_testkit::golden::Golden;
use blurrd_testkit::media::{ClipStatus, MEDIA_ENV, Manifest, MediaDir};

/// The held-out clips, fixed when the manifest was created (plan 11.2).
/// Changing this list invalidates every held-out result.
const HELD_OUT: [&str; 10] = [
    "06.18-20.46",
    "06.21-18.38",
    "06.22-00.38",
    "06.23-13.59",
    "07.15-18.03",
    "07.15-20.54",
    "07.21-13.52",
    "07.25-15.49",
    "07.25-15.50",
    "07.25-16.01",
];

#[test]
fn repository_manifest_is_valid_and_the_split_is_fixed() {
    let m = Manifest::load_repository().unwrap();
    assert_eq!(m.clips.len(), 31);
    let held: Vec<&str> = m
        .clips
        .iter()
        .filter(|c| c.held_out)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(held, HELD_OUT);
    assert_eq!(m.clips.iter().filter(|c| c.soak).count(), 3);
    let ids: Vec<&str> = m.excerpts.iter().map(|e| e.id.as_str()).collect();
    let expected: Vec<String> = (1..=18).map(|i| format!("E{i}")).collect();
    assert_eq!(ids, expected);
    // Both sets cover every HUD region.
    for case in ["killfeed", "banner", "voice", "chat"] {
        for held_out in [false, true] {
            let covered = m
                .excerpts
                .iter()
                .filter(|e| m.is_held_out(e) == held_out)
                .any(|e| e.cases.iter().any(|c| c.starts_with(case)));
            assert!(covered, "{case} held_out={held_out}");
        }
    }
}

#[test]
#[ignore = "needs the recordings in BLURRD_TEST_MEDIA"]
fn real_media_matches_the_manifest() {
    let Some(media) = MediaDir::from_env() else {
        eprintln!("skipping: {MEDIA_ENV} is not set");
        return;
    };
    let m = Manifest::load_repository().unwrap();
    let bad: Vec<String> = m
        .clips
        .iter()
        .filter_map(|c| {
            let status = media.verify(c);
            (status != ClipStatus::Ok).then(|| format!("{}: {status}", c.file))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
    println!("all {} clips match their SHA-256", m.clips.len());
}

#[test]
#[ignore = "needs the goldens in BLURRD_TEST_MEDIA"]
fn real_goldens_match_their_excerpts() {
    let Some(media) = MediaDir::from_env() else {
        eprintln!("skipping: {MEDIA_ENV} is not set");
        return;
    };
    let m = Manifest::load_repository().unwrap();
    let mut found = 0;
    for excerpt in &m.excerpts {
        let path = media.golden_path(excerpt);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let golden = Golden::from_json(&text).unwrap();
        let clip = m.clip(&excerpt.clip).unwrap();
        assert_eq!(golden.excerpt, excerpt.id);
        assert_eq!(golden.clip_sha256, clip.sha256, "{}", excerpt.id);
        assert!(
            excerpt
                .frames
                .iter()
                .any(|r| r.f0 <= golden.frames.f0 && golden.frames.f1 <= r.f1),
            "{}: golden frames outside the excerpt",
            excerpt.id
        );
        found += 1;
    }
    println!("{found} of {} excerpts have a golden", m.excerpts.len());
}
