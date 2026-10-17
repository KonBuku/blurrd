//! `blurrd audit`: read a rendered output with OCR and report every name
//! that is still readable.

use blurrd_core::{Coverage, JobId, VerifyReport, VerifyState};
use blurrd_engine::{AuditReport, AuditRequest, CancellationToken, Engine, cache};
use serde::Serialize;

use crate::Outcome;
use crate::args::AuditArgs;
use crate::output::{Events, leak_report};

/// The final report line of an audit.
#[derive(Debug, Serialize)]
struct Report<'a> {
    #[serde(flatten)]
    audit: &'a AuditReport,
    /// The report itself (frames, regions, boxes and counts; no text).
    report: &'a VerifyReport,
}

/// Audits the output `args.output` of `args.input` rendered with the
/// project `args.plan`.
pub(crate) fn audit(
    args: &AuditArgs,
    engine: &Engine,
    events: &Events,
    cancel: &CancellationToken,
) -> Outcome {
    let job = JobId(uuid::Uuid::now_v7());
    let project = match cache::read_project(&args.plan) {
        Ok(project) => project,
        Err(error) => {
            events.failed(job, &error);
            return Outcome::Failed;
        }
    };
    let mut req = AuditRequest::new(job, args.input.clone(), args.output.clone(), project);
    if args.every_frame {
        req.coverage = Coverage::EveryFrame;
    }
    match engine.audit(&req, events, cancel) {
        Ok(audit) => {
            let mut summary = format!(
                "audited {}: {} frames, {:.1} s\n{}",
                audit.output.display(),
                audit.frames,
                audit.elapsed_ms as f64 / 1000.0,
                leak_report(&audit.report),
            );
            if audit.renamed {
                summary.push_str("\nnames remain readable: the output was renamed *_UNVERIFIED");
            }
            events.report(
                &Report {
                    audit: &audit,
                    report: &audit.report,
                },
                &summary,
            );
            // Exit code 3 only for a readable name; unverifiable items
            // alone leave the output's name and the exit code as they are.
            Outcome::Done {
                unverified: audit.report.summary.state == VerifyState::LeaksRemain,
            }
        }
        Err(error) => Outcome::from_error(&error),
    }
}
