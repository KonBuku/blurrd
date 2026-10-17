//! Jobs and the progress events they emit.
//!
//! The desktop app receives [`JobEvent`]s over an IPC channel; the CLI
//! prints them as JSON lines with `--json`. Both use the same serde form,
//! tagged by `"type"`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::error::AppError;
use crate::region::Region;
use crate::verify::{LeakStatus, VerifyState};

/// Identifier of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct JobId(pub Uuid);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What a job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum JobKind {
    /// Detect, track and verify; writes the project.
    Analyze,
    /// Blur and encode an analysed project.
    Render,
    /// Analyse and render in one streaming pass.
    Run,
    /// Read a rendered output with OCR.
    Audit,
    /// Verify the frame ranges touched by edits again.
    ReverifyRanges,
}

/// A step of a job, in pipeline order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Stage {
    /// Reading the file's streams and metadata.
    Probe,
    /// Decoding frames.
    Decode,
    /// Running the HUD detectors.
    Detect,
    /// Following names over time.
    Track,
    /// Reading text.
    Ocr,
    /// Checking the blurred result.
    Verify,
    /// Blurring and encoding frames.
    BlurEncode,
    /// Writing the output file.
    Mux,
    /// Reading the output with OCR.
    Audit,
}

/// Warnings a job can emit. The UI shows them; none of them fails the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum WarningCode {
    /// The chosen encoder failed; a fallback encoder was used.
    EncoderFallback,
    /// HUD calibration is uncertain; Safe mode was used.
    CalibrationLow,
    /// A region showed HUD structure without names for too long; the span
    /// was covered with slot boxes.
    RegionHealth,
    /// HDR crops were tone-mapped (or Safe mode was used).
    HdrToneMapped,
    /// Names were seen outside every region (for example name tags above
    /// heads). Information only: they are out of scope and not blurred.
    OutOfScopeNames,
    /// OCR could not read some names on the source; they are covered only
    /// by the detector boxes.
    UnverifiableItems,
    /// A user edit no longer matches any box and is inactive.
    OverrideUnbound,
    /// OCR is not available; the result is unverified.
    OcrUnavailable,
    /// The thumbnails or the preview proxy could not be made; the preview
    /// may not show this video.
    PreviewUnavailable,
}

/// Result of a finished job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct JobSummary {
    /// Frames processed.
    pub frames: u32,
    /// Wall-clock time in milliseconds.
    pub elapsed_ms: u64,
    /// The written file, for render and run jobs.
    pub output: Option<PathBuf>,
    /// Verification result, if the job verified.
    pub verify: Option<VerifyState>,
    /// Number of warnings emitted.
    pub warnings: u32,
}

/// A progress event of a job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum JobEvent {
    /// The job started.
    Started {
        /// The job.
        job: JobId,
        /// What it does.
        kind: JobKind,
        /// Frames it will process in each pass.
        total_frames: u32,
        /// Passes over the video it plans; each one counts its frames
        /// from 0 again. A run analyses and renders in one pass, or in
        /// two when the project has edits (the render needs the finished
        /// tracks). A run whose verification adds fix boxes renders once
        /// more, in a pass beyond this count.
        passes: u32,
    },
    /// The job entered a stage.
    Stage {
        /// The job.
        job: JobId,
        /// The stage.
        stage: Stage,
        /// The pass the stage belongs to, from 0.
        pass: u32,
    },
    /// Progress, at most 20 times per second.
    Progress {
        /// The job.
        job: JobId,
        /// Frames done.
        frame: u32,
        /// Frames in total.
        total: u32,
        /// Current speed in frames per second.
        fps: f32,
        /// Estimated time left in milliseconds.
        eta_ms: u64,
    },
    /// The verification found a readable name.
    LeakFound {
        /// The job.
        job: JobId,
        /// First frame of the leak.
        frame: u32,
        /// Region of the leak.
        region: Region,
        /// What happened to it.
        status: LeakStatus,
    },
    /// Something the user should know.
    Warning {
        /// The job.
        job: JobId,
        /// What happened.
        code: WarningCode,
        /// One-line explanation.
        message: String,
        /// Affected frames, if any.
        frames: Option<(u32, u32)>,
    },
    /// The job finished.
    Finished {
        /// The job.
        job: JobId,
        /// Its result.
        summary: JobSummary,
    },
    /// The job failed.
    Failed {
        /// The job.
        job: JobId,
        /// Why.
        error: AppError,
    },
    /// The job was canceled.
    Canceled {
        /// The job.
        job: JobId,
    },
}

impl JobEvent {
    /// The job the event belongs to.
    pub const fn job(&self) -> JobId {
        match *self {
            Self::Started { job, .. }
            | Self::Stage { job, .. }
            | Self::Progress { job, .. }
            | Self::LeakFound { job, .. }
            | Self::Warning { job, .. }
            | Self::Finished { job, .. }
            | Self::Failed { job, .. }
            | Self::Canceled { job } => job,
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{JobEvent, JobId, Stage};

    #[test]
    fn events_are_tagged_by_type() {
        let job = JobId(Uuid::from_u128(7));
        let event = JobEvent::Stage {
            job,
            stage: Stage::Detect,
            pass: 1,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "type": "Stage", "job": "00000000-0000-0000-0000-000000000007",
                                "stage": "Detect", "pass": 1 })
        );
        assert_eq!(serde_json::from_value::<JobEvent>(json).unwrap().job(), job);
    }
}
