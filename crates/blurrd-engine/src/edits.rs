//! Keeping the user's edits when a job saves its result.
//!
//! An analysis or a verification of edited ranges reads the project's
//! edits when it starts and saves the project when it ends, minutes
//! later. The desktop app saves edits in the meantime. A job therefore
//! saves through [`crate::cache::update_project`] and joins its result
//! with the project stored at that moment: the edits stored then win, and
//! the frames of every edit the job did not see join the unverified
//! ranges, because its verification did not check them.
//!
//! A re-analysis replaces every track. The edits carry no track ids:
//! `Suppress` and `Adjust` bind again by overlap with the new tracks each
//! time a plan is resolved (`blurrd_core::BlurPlan::bindings`). The ones
//! that match no new box stay in the project, inactive, and the analysis
//! warns about each ([`warn_unbound`]) so the user can delete or redo it.

use blurrd_core::{BindStatus, FrameRange, Project, WarningCode, touched_ranges};

use crate::job::Job;

/// Joins `project`, the result of a job that read the stored project at
/// revision `base` (`None`: there was none), with `current`, the project
/// stored now.
///
/// Unchanged since `base`, the result is stored as it is. Otherwise it
/// takes the stored edits, a revision after the stored one, and the
/// frames of the edits that differ from the ones it was made with as
/// unverified ranges; its verification stays current when no edit
/// differs.
pub(crate) fn keep_current_edits(
    mut project: Project,
    base: Option<u64>,
    current: Option<Project>,
) -> Project {
    let Some(current) = current else {
        return project;
    };
    if base == Some(current.revision) {
        return project;
    }
    let verified_current = project.verify.plan_revision == project.revision;
    project.revision = project.revision.max(current.revision + 1);
    let touched = touched_ranges(&project.overrides, &current.overrides);
    if touched.is_empty() {
        if verified_current {
            project.verify.plan_revision = project.revision;
        }
    } else {
        let mut ranges = std::mem::take(&mut project.verify.unverified_ranges);
        ranges.extend(touched);
        project.verify.unverified_ranges = FrameRange::merge_all(ranges);
    }
    project.overrides = current.overrides;
    project
}

/// Warns about every edit of `project` that binds to no box of its
/// tracks: it is inactive until a box matches it again or the user
/// deletes it.
pub(crate) fn warn_unbound(project: &Project, job: &Job<'_>) {
    let bindings = project.plan().bindings();
    for (edit, binding) in project.overrides.iter().zip(bindings) {
        if binding.status == BindStatus::Unbound {
            let (f0, f1) = edit.frames();
            job.warning(
                WarningCode::OverrideUnbound,
                format!(
                    "A box you changed or hid on frames {f0}-{f1} no longer matches a \
                     detected name; the edit is inactive."
                ),
                Some((f0, f1)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{
        BlurParams, DetectorInfo, Fingerprint, FrameRange, HudGeometry, Mode, Override, Project,
        ProjectMedia, Rational, RectI, Region, RegionSet, Sensitivity, TemporalParams, VerifyState,
    };
    use uuid::Uuid;

    use super::keep_current_edits;

    fn project(revision: u64) -> Project {
        let media = ProjectMedia {
            path: "clip.mp4".into(),
            fingerprint: Fingerprint {
                size: 1,
                mtime: 0,
                blake3_head_tail: "00".repeat(32),
            },
            width: 1920,
            height: 1080,
            fps: Rational::new(60, 1).unwrap(),
            time_base: Rational::new(1, 60).unwrap(),
            frame_count: 600,
            pts_sha256: "00".repeat(32),
            is_vfr: false,
            has_b_frames: false,
            keyframes_sha256: "00".repeat(32),
        };
        let detector = DetectorInfo {
            version: "detect/test".into(),
            regions: RegionSet::all(),
            mode: Mode::Tight,
            sensitivity: Sensitivity::default(),
            temporal: TemporalParams::default(),
        };
        let mut project = Project::new(
            media,
            HudGeometry::default(),
            detector,
            BlurParams::default(),
        );
        project.revision = revision;
        project.verify.plan_revision = revision;
        project.verify.state = VerifyState::Clean;
        project
    }

    fn add(uid: u128, f0: u32, f1: u32) -> Override {
        Override::Add {
            uid: Uuid::from_u128(uid),
            region: Region::Manual,
            f0,
            f1,
            rect: RectI::new(10, 10, 50, 30).unwrap(),
        }
    }

    #[test]
    fn a_result_without_concurrent_edits_is_stored_as_it_is() {
        let mut result = project(4);
        result.overrides = vec![add(1, 0, 9)];
        let mut stored = project(3);
        stored.overrides = vec![add(1, 0, 9)];
        assert_eq!(
            keep_current_edits(result.clone(), Some(3), Some(stored)),
            result
        );
        assert_eq!(keep_current_edits(result.clone(), None, None), result);
    }

    #[test]
    fn edits_saved_while_the_job_ran_stay_and_are_unverified() {
        // The job read revision 3 with one edit and verified revision 4.
        let mut result = project(4);
        result.overrides = vec![add(1, 0, 9)];
        // Meanwhile the user moved that edit and added one (revision 5).
        let mut stored = project(5);
        stored.overrides = vec![add(1, 20, 29), add(2, 100, 110)];
        let joined = keep_current_edits(result, Some(3), Some(stored.clone()));
        assert_eq!(joined.overrides, stored.overrides);
        assert_eq!(joined.revision, 6);
        assert_eq!(joined.verify.plan_revision, 4);
        assert_eq!(
            joined.verify.unverified_ranges,
            vec![
                FrameRange::new(0, 9).unwrap(),
                FrameRange::new(20, 29).unwrap(),
                FrameRange::new(100, 110).unwrap()
            ]
        );
        assert!(!crate::is_verified(&joined));
    }

    #[test]
    fn a_saved_edit_that_was_undone_keeps_the_verification_current() {
        let mut result = project(4);
        result.overrides = vec![add(1, 0, 9)];
        // Two saves while the job ran: a change, then its undo.
        let mut stored = project(5);
        stored.overrides = vec![add(1, 0, 9)];
        let joined = keep_current_edits(result, Some(3), Some(stored));
        assert_eq!(joined.revision, 6);
        assert_eq!(joined.verify.plan_revision, 6);
        assert!(crate::is_verified(&joined));
    }
}
