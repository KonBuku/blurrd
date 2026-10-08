//! Drawing HUD elements in reference coordinates while recording their
//! ground truth.

use blurrd_core::{NameKind, RectI, Region};

use crate::font::{FontSet, TextBitmap, TextStyle};
use crate::image::RgbFrame;
use crate::layout::{Anchor, Layout};
use crate::mask::Mask;
use crate::truth::{Content, ContentKind, FrameTruth, GLYPH_HALO_REF_PX, ItemId, TruthItem};

/// Opacity of the drop shadow under HUD text.
const SHADOW_ALPHA: f32 = 0.45;

/// Text rendered once for a layout, with its width in reference pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Label {
    /// The rendered text (source pixels).
    pub(crate) bitmap: TextBitmap,
    /// The pen advance in reference pixels.
    pub(crate) width: f32,
}

impl Label {
    /// Renders `text` with capitals `cap_ref` reference pixels tall.
    pub(crate) fn new(fonts: &FontSet, layout: &Layout, text: &str, cap_ref: f32) -> Self {
        let style = TextStyle {
            cap_px: cap_ref * layout.sy,
            x_scale: layout.sx / layout.sy,
        };
        let bitmap = fonts.render(text, style);
        let width = bitmap.advance / layout.sx;
        Self { bitmap, width }
    }
}

/// A frame being painted, its layout and the truth collected so far.
pub(crate) struct Painter<'a> {
    pub(crate) frame: &'a mut RgbFrame,
    pub(crate) layout: &'a Layout,
    pub(crate) truth: &'a mut FrameTruth,
}

/// How a name is drawn.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NameDraw {
    pub(crate) id: ItemId,
    pub(crate) region: Region,
    pub(crate) kind: NameKind,
    /// Slot box in reference pixels.
    pub(crate) slot: RectI,
}

impl Painter<'_> {
    /// Maps a reference rectangle.
    pub(crate) fn map(&self, r: RectI, anchor: Anchor) -> RectI {
        self.layout.rect(r, anchor)
    }

    /// Fills a reference rectangle.
    pub(crate) fn fill(&mut self, r: RectI, anchor: Anchor, rgb: [u8; 3], alpha: f32) -> RectI {
        let src = self.map(r, anchor);
        self.frame.fill_rect(src, rgb, alpha);
        src
    }

    /// Fills a reference rectangle with a rounded rectangle.
    pub(crate) fn rounded(
        &mut self,
        r: RectI,
        anchor: Anchor,
        radius_ref: f32,
        rgb: [u8; 3],
        alpha: f32,
    ) -> RectI {
        let src = self.map(r, anchor);
        self.frame
            .fill_rounded(src, radius_ref * self.layout.sy, rgb, alpha);
        src
    }

    /// Records non-name content at a reference rectangle.
    pub(crate) fn content(&mut self, kind: ContentKind, r: RectI, anchor: Anchor) {
        let rect = self.map(r, anchor);
        self.content_src(kind, rect);
    }

    /// Records non-name content at a source rectangle (clipped to the
    /// frame; dropped if outside).
    pub(crate) fn content_src(&mut self, kind: ContentKind, rect: RectI) {
        if let Some(rect) = rect.intersection(&self.frame.bounds()) {
            self.truth.content.push(Content { kind, rect });
        }
    }

    /// Draws a label with its pen at reference `(x, baseline)` and a drop
    /// shadow, clipped to `clip` (reference) if given. Returns the ink mask
    /// (glyphs and shadow, before any halo) in source pixels.
    pub(crate) fn text(
        &mut self,
        label: &Label,
        pen: (f32, f32),
        anchor: Anchor,
        rgb: [u8; 3],
        alpha: f32,
        clip: Option<RectI>,
    ) -> Option<Mask> {
        if alpha <= 0.0 {
            return None;
        }
        let bmp = &label.bitmap;
        let x0 = self.layout.x(pen.0, anchor).round() as i32 - bmp.origin_x;
        let y0 = self.layout.y(pen.1).round() as i32 - bmp.baseline;
        let shadow = self.layout.len_px(1.0) as i32;
        let bounds = self.frame.bounds();
        let clip = match clip {
            Some(c) => self.map(c, anchor).intersection(&bounds)?,
            None => bounds,
        };
        let area = RectI::new(
            x0,
            y0,
            x0 + bmp.width as i32 + shadow,
            y0 + bmp.height as i32 + shadow,
        )
        .ok()?
        .intersection(&clip)?;
        let mut ink = Mask::empty(area);
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let glyph = bmp.at(x - x0, y - y0);
                let shade = bmp.at(x - x0 - shadow, y - y0 - shadow);
                if shade > 0.0 {
                    self.frame
                        .blend(x, y, [0, 0, 0], shade * SHADOW_ALPHA * alpha);
                }
                if glyph > 0.0 {
                    self.frame.blend(x, y, rgb, glyph * alpha);
                }
                if glyph > 0.0 || shade > 0.0 {
                    ink.insert(x, y);
                }
            }
        }
        (ink.count() > 0).then_some(ink)
    }

    /// Draws a player name and records it as a truth item.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors `text` plus the item description"
    )]
    pub(crate) fn name(
        &mut self,
        draw: NameDraw,
        label: &Label,
        pen: (f32, f32),
        anchor: Anchor,
        rgb: [u8; 3],
        alpha: f32,
        clip: Option<RectI>,
    ) {
        let Some(ink) = self.text(label, pen, anchor, rgb, alpha, clip) else {
            return;
        };
        let Some(text) = ink.bounds() else {
            return;
        };
        let halo = self.layout.len_px(GLYPH_HALO_REF_PX);
        let glyphs = ink.dilated(halo);
        let slot_src = self.map(draw.slot, anchor);
        // The slot always contains the ink and its halo, even
        // mid-animation, so a slot box never leaks.
        let reach = glyphs.bounds().unwrap_or(text);
        let slot = RectI {
            x0: slot_src.x0.min(reach.x0),
            y0: slot_src.y0.min(reach.y0),
            x1: slot_src.x1.max(reach.x1),
            y1: slot_src.y1.max(reach.y1),
        };
        self.truth.items.push(TruthItem {
            id: draw.id,
            region: draw.region,
            kind: draw.kind,
            text,
            glyphs,
            slot,
            opacity: alpha,
        });
    }

    /// Draws text that is not a name and records it as content.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors `text` plus the content kind"
    )]
    pub(crate) fn label(
        &mut self,
        kind: ContentKind,
        label: &Label,
        pen: (f32, f32),
        anchor: Anchor,
        rgb: [u8; 3],
        alpha: f32,
        clip: Option<RectI>,
    ) {
        if let Some(rect) = self
            .text(label, pen, anchor, rgb, alpha, clip)
            .and_then(|ink| ink.bounds())
        {
            self.content_src(kind, rect);
        }
    }

    /// Draws textured art (a portrait) into a reference rectangle and
    /// records it as content.
    pub(crate) fn portrait(&mut self, r: RectI, anchor: Anchor, seed: u64, alpha: f32) {
        let src = self.map(r, anchor);
        self.frame.for_each_in(src, |f, x, y| {
            let h = crate::rng::hash2(x / 3, y / 3, seed);
            let rgb = [
                (60 + (h & 0x7F)) as u8,
                (50 + ((h >> 8) & 0x7F)) as u8,
                (70 + ((h >> 16) & 0x7F)) as u8,
            ];
            f.blend(x, y, rgb, alpha);
        });
        self.content_src(ContentKind::Portrait, src);
    }
}

/// A reference rectangle from floating-point edges, rounded outward.
pub(crate) fn rect_f(x0: f32, y0: f32, x1: f32, y1: f32) -> RectI {
    let (x0, y0) = (x0.floor() as i32, y0.floor() as i32);
    RectI {
        x0,
        y0,
        x1: (x1.ceil() as i32).max(x0 + 1),
        y1: (y1.ceil() as i32).max(y0 + 1),
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{NameKind, Region};

    use super::{Label, NameDraw, Painter, rect_f};
    use crate::font::FontSet;
    use crate::image::RgbFrame;
    use crate::layout::{Anchor, Layout};
    use crate::truth::{FrameTruth, ItemId};

    #[test]
    fn records_dilated_glyphs_inside_the_slot() {
        let fonts = FontSet::latin();
        let layout = Layout::reference();
        let mut frame = RgbFrame::filled(1920, 1080, [40, 40, 40]);
        let mut truth = FrameTruth::default();
        let label = Label::new(&fonts, &layout, "Kovax", 14.0);
        let mut p = Painter {
            frame: &mut frame,
            layout: &layout,
            truth: &mut truth,
        };
        p.name(
            NameDraw {
                id: ItemId(1),
                region: Region::KillFeed,
                kind: NameKind::Victim,
                slot: rect_f(1700.0, 101.0, 1836.0, 124.0),
            },
            &label,
            (1790.0, 120.0),
            Anchor::Right,
            [245, 245, 245],
            1.0,
            None,
        );
        let item = &truth.items[0];
        let text = item.text;
        assert!(text.height() >= 14 && text.height() <= 20, "{text}");
        assert_eq!(item.glyphs.bounds().unwrap().x0, text.x0 - 2);
        assert!(item.slot.x0 <= text.x0 && item.slot.x1 >= text.x1);
        // Every glyph pixel was painted brighter than the background.
        let lit = item
            .glyphs
            .pixels()
            .filter(|&(x, y)| frame.pixel(x, y).unwrap()[0] > 100)
            .count();
        assert!(lit > 40);
    }

    #[test]
    fn clips_and_skips_invisible_text() {
        let fonts = FontSet::latin();
        let layout = Layout::reference();
        let mut frame = RgbFrame::filled(64, 32, [0, 0, 0]);
        let mut truth = FrameTruth::default();
        let label = Label::new(&fonts, &layout, "Abc", 12.0);
        let mut p = Painter {
            frame: &mut frame,
            layout: &layout,
            truth: &mut truth,
        };
        assert!(
            p.text(&label, (4.0, 20.0), Anchor::Left, [255; 3], 0.0, None)
                .is_none()
        );
        let clipped = p
            .text(
                &label,
                (4.0, 20.0),
                Anchor::Left,
                [255; 3],
                1.0,
                Some(rect_f(0.0, 0.0, 8.0, 32.0)),
            )
            .unwrap();
        assert!(clipped.bounds().unwrap().x1 <= 8);
    }
}
