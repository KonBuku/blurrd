//! Results of the OCR verification.
//!
//! Reports never contain OCR text or values derived from it: only frames,
//! regions, boxes, confidences and counts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::geometry::RectI;
use crate::media::Fingerprint;
use crate::region::Region;
use crate::schema::{Document, SchemaError};
use crate::settings::Coverage;
use crate::time::FrameRange;

/// Overall verification result of a project or report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum VerifyState {
    /// Nothing readable remains and every item was checked.
    Clean,
    /// Not (fully) checked: never verified, edited since, or items OCR could
    /// not read on the source. Outputs are named `*_UNVERIFIED` until the
    /// user accepts the report.
    #[default]
    Unverified,
    /// At least one name is still readable.
    LeaksRemain,
}

/// How a leak was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum LeakKind {
    /// The read text matches a name seen elsewhere in the video.
    Dictionary,
    /// Text was detected or read with enough confidence in a name slot.
    DictionaryFree,
    /// A known name outside every supported region (in practice a name tag
    /// above a player's head). Information only: never blurred, never
    /// counted.
    OutOfScope,
}

/// What happened to a leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum LeakStatus {
    /// Covered by a fix box and checked again.
    Fixed,
    /// Still readable.
    Open,
    /// Still readable; the user accepted the report.
    Accepted,
    /// Information only (out-of-scope names).
    Info,
}

/// Where in the frame a leak was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum LeakZone {
    /// Inside a name slot of a region.
    NameSlot,
    /// Inside a region's protected zone, outside the known name slots.
    ProtectedZone,
    /// In the full-frame sample, outside every protected zone.
    FullFrame,
}

/// One leak, merged over the frames on which it was readable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Leak {
    /// Frames on which the text was readable.
    pub frame_range: FrameRange,
    /// The region, or `None` outside every region.
    pub region: Option<Region>,
    /// Where it was found.
    pub zone: LeakZone,
    /// Where the text was, in source pixels.
    pub rect: RectI,
    /// Detection or recognition confidence, 0.0 to 1.0.
    pub conf: f32,
    /// How it was found.
    pub kind: LeakKind,
    /// What happened to it.
    pub status: LeakStatus,
    /// Optional deduplication key: an 8-character HMAC with a random
    /// per-install key. Present only when deduplication is enabled.
    pub dedup: Option<String>,
}

/// Verification counts of one region.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RegionVerify {
    /// Fraction of items whose name OCR read on the clean source, 0.0 to 1.0.
    pub positive_control: f32,
    /// Number of items (names over their lifetime) checked.
    pub items: u32,
    /// Items OCR could not read on the source; covered only by the
    /// geometric guarantee.
    pub unverifiable_items: u32,
}

/// Counts derived from the leaks and regions of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VerifySummary {
    /// The result.
    pub state: VerifyState,
    /// Leaks still readable.
    pub open: u32,
    /// Leaks fixed.
    pub fixed: u32,
    /// Leaks the user accepted.
    pub accepted: u32,
    /// Out-of-scope names listed as information.
    pub out_of_scope: u32,
    /// Items OCR could not read on the source, over all regions.
    pub unverifiable_items: u32,
}

impl VerifySummary {
    /// Computes the summary.
    ///
    /// The state is `LeaksRemain` with any open leak, otherwise
    /// `Unverified` with any unverifiable item, otherwise `Clean`.
    /// Out-of-scope entries never change the state.
    pub fn compute(leaks: &[Leak], per_region: &BTreeMap<Region, RegionVerify>) -> Self {
        let count = |status: LeakStatus| {
            leaks
                .iter()
                .filter(|l| l.kind != LeakKind::OutOfScope && l.status == status)
                .count() as u32
        };
        let open = count(LeakStatus::Open);
        let unverifiable_items = per_region.values().map(|r| r.unverifiable_items).sum();
        let state = if open > 0 {
            VerifyState::LeaksRemain
        } else if unverifiable_items > 0 {
            VerifyState::Unverified
        } else {
            VerifyState::Clean
        };
        Self {
            state,
            open,
            fixed: count(LeakStatus::Fixed),
            accepted: count(LeakStatus::Accepted),
            out_of_scope: leaks
                .iter()
                .filter(|l| l.kind == LeakKind::OutOfScope)
                .count() as u32,
            unverifiable_items,
        }
    }
}

/// A verification report (`verify-*.json`, schema `blurrd-verify/1`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VerifyReport {
    /// Always `blurrd-verify/1`.
    pub schema: String,
    /// Revision within the schema's major version.
    pub version: u32,
    /// Project revision the report checked.
    pub plan_revision: u64,
    /// The source file.
    pub source_fingerprint: Fingerprint,
    /// SHA-256 of the audited output, for output audits.
    pub output_sha256: Option<String>,
    /// Which frames were read.
    pub coverage: Coverage,
    /// Number of frames read.
    pub frames_checked: u32,
    /// Counts per region.
    pub per_region: BTreeMap<Region, RegionVerify>,
    /// Leaks, in frame order.
    pub leaks: Vec<Leak>,
    /// Derived counts; always equal to [`VerifySummary::compute`].
    pub summary: VerifySummary,
}

impl VerifyReport {
    /// Creates a report and computes its summary.
    pub fn new(
        plan_revision: u64,
        source_fingerprint: Fingerprint,
        coverage: Coverage,
        frames_checked: u32,
        per_region: BTreeMap<Region, RegionVerify>,
        leaks: Vec<Leak>,
    ) -> Self {
        let summary = VerifySummary::compute(&leaks, &per_region);
        Self {
            schema: Self::schema_id(),
            version: Self::REVISION,
            plan_revision,
            source_fingerprint,
            output_sha256: None,
            coverage,
            frames_checked,
            per_region,
            leaks,
            summary,
        }
    }
}

impl Document for VerifyReport {
    const NAME: &'static str = "blurrd-verify";
    const MAJOR: u32 = 1;
    const REVISION: u32 = 1;

    fn validate(&self) -> Result<(), SchemaError> {
        for leak in &self.leaks {
            let out_of_scope = leak.kind == LeakKind::OutOfScope;
            if out_of_scope != (leak.status == LeakStatus::Info) {
                return Err(SchemaError::Invalid(format!(
                    "leak at frames {}..={}: out-of-scope entries, and only those, have status Info",
                    leak.frame_range.f0, leak.frame_range.f1
                )));
            }
            if out_of_scope != leak.region.is_none() {
                return Err(SchemaError::Invalid(format!(
                    "leak at frames {}..={}: only out-of-scope entries lie outside every region",
                    leak.frame_range.f0, leak.frame_range.f1
                )));
            }
        }
        if self.summary != VerifySummary::compute(&self.leaks, &self.per_region) {
            return Err(SchemaError::Invalid(
                "the summary does not match the leaks".to_owned(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Leak, LeakKind, LeakStatus, LeakZone, RegionVerify, VerifyReport, VerifyState};
    use crate::geometry::RectI;
    use crate::media::Fingerprint;
    use crate::region::Region;
    use crate::schema::{self, SchemaError};
    use crate::settings::Coverage;
    use crate::time::FrameRange;

    fn leak(region: Option<Region>, kind: LeakKind, status: LeakStatus) -> Leak {
        Leak {
            frame_range: FrameRange::new(100, 140).unwrap(),
            region,
            zone: if region.is_some() {
                LeakZone::NameSlot
            } else {
                LeakZone::FullFrame
            },
            rect: RectI::new(900, 300, 980, 318).unwrap(),
            conf: 0.8,
            kind,
            status,
            dedup: None,
        }
    }

    fn report(leaks: Vec<Leak>, unverifiable: u32) -> VerifyReport {
        let per_region = BTreeMap::from([(
            Region::KillFeed,
            RegionVerify {
                positive_control: 0.95,
                items: 40,
                unverifiable_items: unverifiable,
            },
        )]);
        let fingerprint = Fingerprint {
            size: 1,
            mtime: 0,
            blake3_head_tail: "00".repeat(32),
        };
        VerifyReport::new(3, fingerprint, Coverage::Structure, 3601, per_region, leaks)
    }

    #[test]
    fn out_of_scope_names_never_change_the_result() {
        let r = report(vec![leak(None, LeakKind::OutOfScope, LeakStatus::Info)], 0);
        assert_eq!(r.summary.state, VerifyState::Clean);
        assert_eq!(r.summary.out_of_scope, 1);
        let json = schema::to_json(&r).unwrap();
        assert_eq!(schema::from_json::<VerifyReport>(&json).unwrap(), r);
    }

    #[test]
    fn open_leaks_and_unverifiable_items_set_the_state() {
        let open = leak(
            Some(Region::KillFeed),
            LeakKind::Dictionary,
            LeakStatus::Open,
        );
        assert_eq!(
            report(vec![open], 0).summary.state,
            VerifyState::LeaksRemain
        );
        let fixed = leak(
            Some(Region::KillFeed),
            LeakKind::DictionaryFree,
            LeakStatus::Fixed,
        );
        assert_eq!(
            report(vec![fixed.clone()], 0).summary.state,
            VerifyState::Clean
        );
        assert_eq!(
            report(vec![fixed], 2).summary.state,
            VerifyState::Unverified
        );
    }

    #[test]
    fn rejects_inconsistent_reports() {
        let wrong = report(vec![leak(None, LeakKind::OutOfScope, LeakStatus::Open)], 0);
        assert!(matches!(
            schema::to_json(&wrong),
            Err(SchemaError::Invalid(_))
        ));
        let mut stale = report(Vec::new(), 0);
        stale.summary.open = 5;
        assert!(matches!(
            schema::to_json(&stale),
            Err(SchemaError::Invalid(_))
        ));
    }
}
