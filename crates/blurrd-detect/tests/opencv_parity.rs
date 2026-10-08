//! HSV parity with OpenCV.
//!
//! `tests/data/opencv_hsv.csv` was generated once, outside the repository,
//! with OpenCV 4.11.0: `cv2.cvtColor(bgr, cv2.COLOR_BGR2HSV)` on every
//! combination of 16 levels per channel plus 2048 seeded random colours.
//! The ignored test compares all 16,777,216 colours against a full table
//! written the same way (H, S, V bytes per colour, index
//! `(r << 16) | (g << 8) | b`), whose path is given in
//! `BLURRD_OPENCV_HSV_TABLE`.

use blurrd_detect::hsv;

#[test]
fn hsv_matches_the_opencv_table_exactly() {
    let table = include_str!("data/opencv_hsv.csv");
    let mut rows = 0;
    for line in table.lines().filter(|l| !l.starts_with('#')).skip(1) {
        let v: Vec<u8> = line.split(',').map(|f| f.parse().unwrap()).collect();
        assert_eq!(v.len(), 6, "{line}");
        assert_eq!(
            hsv([v[0], v[1], v[2]]),
            [v[3], v[4], v[5]],
            "rgb {:?}",
            &v[..3]
        );
        rows += 1;
    }
    assert_eq!(rows, 16 * 16 * 16 + 2048);
}

#[test]
#[ignore = "needs the full OpenCV table in BLURRD_OPENCV_HSV_TABLE"]
fn real_hsv_matches_opencv_for_every_colour() {
    let path = std::env::var("BLURRD_OPENCV_HSV_TABLE").expect("BLURRD_OPENCV_HSV_TABLE");
    let table = std::fs::read(path).unwrap();
    assert_eq!(table.len(), 3 << 24);
    let mut mismatches = 0u64;
    for (i, expected) in table.as_chunks::<3>().0.iter().enumerate() {
        let rgb = [(i >> 16) as u8, (i >> 8) as u8, i as u8];
        if hsv(rgb) != *expected {
            mismatches += 1;
        }
    }
    println!("checked {} colours, {mismatches} mismatches", 1u32 << 24);
    assert_eq!(mismatches, 0);
}
