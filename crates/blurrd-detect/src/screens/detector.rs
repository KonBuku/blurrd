//! All screen surfaces of a video together.

use std::collections::VecDeque;

use blurrd_core::{Region, RegionSet};

use super::gate::{GateSignal, Gates};
use super::gate_spec;
use super::match_end::MatchEndReader;
use super::recap::RecapReader;
use super::sample::{FrameView, ScreenMap};
use super::scoreboard::ScoreboardReader;
use super::spectating::SpectatingReader;
use super::temporal::{Slot, SurfaceFrame, SurfaceTrack};
use crate::detection::Detection;

/// The screen surfaces in a fixed order ([`Region::SCREENS`]).
const SURFACES: [Region; 4] = Region::SCREENS;

/// Gate signals of the four surfaces of one frame, in
/// [`Region::SCREENS`] order (disabled surfaces are never candidates).
pub type ScreenSignals = [GateSignal; 4];

/// Finds the names on every enabled screen surface of a video.
///
/// Feed the frames in order. Per frame, [`ScreenDetector::gate`] reads
/// the quarter-scale frame; for every candidate surface,
/// [`ScreenDetector::read`] reads its native zone
/// ([`super::zone`]); [`ScreenDetector::push`] hands both to the temporal
/// stage and returns the frames that are decided, with their boxes, at
/// most [`super::temporal::LOOKAHEAD`] frames later.
/// [`ScreenDetector::detect`] does all three for a caller that has the
/// whole frame at both scales.
#[derive(Debug, Clone)]
pub struct ScreenDetector {
    enabled: [bool; 4],
    gates: Gates,
    scoreboard: ScoreboardReader,
    recap: RecapReader,
    spectating: SpectatingReader,
    match_end: MatchEndReader,
    tracks: [SurfaceTrack; 4],
    /// Decided frames per surface, not yet merged.
    decided: [VecDeque<(u32, Vec<Detection>)>; 4],
    scratch: Vec<(u32, Vec<Detection>)>,
}

impl ScreenDetector {
    /// A detector for the screen surfaces in `regions` (HUD regions are
    /// ignored).
    pub fn new(regions: &RegionSet) -> Self {
        Self {
            enabled: SURFACES.map(|r| regions.contains(r)),
            gates: Gates::new(),
            scoreboard: ScoreboardReader::new(),
            recap: RecapReader::new(),
            spectating: SpectatingReader::new(),
            match_end: MatchEndReader::new(),
            tracks: SURFACES.map(SurfaceTrack::new),
            decided: Default::default(),
            scratch: Vec::new(),
        }
    }

    /// The gate signals of one frame from its quarter-scale view.
    pub fn gate(&mut self, quarter: &FrameView<'_>, map: &ScreenMap) -> ScreenSignals {
        let mut out = [GateSignal::default(); 4];
        for (i, region) in SURFACES.into_iter().enumerate() {
            if let (true, Some(spec)) = (self.enabled[i], gate_spec(region)) {
                out[i] = self.gates.evaluate(spec, quarter, map);
            }
        }
        out
    }

    /// The name slots of `region` in `native` (a native-scale view that
    /// contains the surface's zone), or `None` if the surface's structure
    /// is not found or `region` is not a screen surface.
    pub fn read(
        &mut self,
        region: Region,
        native: &FrameView<'_>,
        map: &ScreenMap,
        signal: &GateSignal,
    ) -> Option<Vec<Slot>> {
        match region {
            Region::Scoreboard => self.scoreboard.read(native, map, signal),
            Region::DeathRecap => self.recap.read(native, map),
            Region::Spectating => self.spectating.read(native, map),
            Region::MatchEnd => self.match_end.read(native, map, signal),
            _ => None,
        }
    }

    /// Adds frame `frame` (frames in order, without gaps) with what each
    /// surface showed, in [`Region::SCREENS`] order, and appends every
    /// frame that is now decided for all surfaces to `out` as `(frame,
    /// boxes)`.
    pub fn push(
        &mut self,
        frame: u32,
        surfaces: [SurfaceFrame; 4],
        out: &mut Vec<(u32, Vec<Detection>)>,
    ) {
        for (i, data) in surfaces.into_iter().enumerate() {
            self.scratch.clear();
            self.tracks[i].push(frame, data, &mut self.scratch);
            self.decided[i].extend(self.scratch.drain(..));
        }
        self.merge(out);
    }

    /// Gates, reads and pushes one frame given at both scales.
    pub fn detect(
        &mut self,
        frame: u32,
        quarter: &FrameView<'_>,
        native: &FrameView<'_>,
        map: &ScreenMap,
        out: &mut Vec<(u32, Vec<Detection>)>,
    ) {
        let signals = self.gate(quarter, map);
        let mut surfaces: [SurfaceFrame; 4] = Default::default();
        for (i, region) in SURFACES.into_iter().enumerate() {
            let signal = signals[i];
            surfaces[i].candidate = signal.candidate;
            if signal.candidate {
                surfaces[i].slots = self.read(region, native, map, &signal);
            }
        }
        self.push(frame, surfaces, out);
    }

    /// Decides every pending frame (end of the video).
    pub fn finish(mut self, out: &mut Vec<(u32, Vec<Detection>)>) {
        let tracks = std::mem::replace(&mut self.tracks, SURFACES.map(SurfaceTrack::new));
        for (i, track) in tracks.into_iter().enumerate() {
            self.scratch.clear();
            track.finish(&mut self.scratch);
            self.decided[i].extend(self.scratch.drain(..));
        }
        self.merge(out);
    }

    /// Moves the frames every surface has decided to `out`.
    fn merge(&mut self, out: &mut Vec<(u32, Vec<Detection>)>) {
        loop {
            let Some(frame) = self.decided[0].front().map(|d| d.0) else {
                return;
            };
            if self
                .decided
                .iter()
                .any(|d| d.front().is_none_or(|f| f.0 != frame))
            {
                return;
            }
            let mut boxes = Vec::new();
            for (i, queue) in self.decided.iter_mut().enumerate() {
                if let Some((_, b)) = queue.pop_front()
                    && self.enabled[i]
                {
                    boxes.extend(b);
                }
            }
            out.push((frame, boxes));
        }
    }
}
