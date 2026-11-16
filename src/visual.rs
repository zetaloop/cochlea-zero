use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::Arc,
};

use gpui::{
    Bounds, Context, Hsla, IntoElement, PathBuilder, Pixels, Render, RenderImage, Window,
    assets::IconName,
    canvas,
    component::{ActiveTheme, Icon},
    div, fill, point,
    prelude::*,
    px, rgb, size,
};
use image::{Frame, RgbaImage};

use crate::{
    analysis::{Analyzer, Viewport},
    audio::Track,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Spectrum,
    Waveform,
    Combined,
}

const TILE_WIDTH: i64 = 256 - 2;

#[derive(PartialEq)]
struct Format {
    width: usize,
    height: usize,
    samples: f64,
    colors: [[u8; 3]; 2],
}

struct Signal {
    track: Arc<Track>,
    analyzer: Analyzer,
    images: BTreeMap<i64, Arc<RenderImage>>,
    format: Option<Format>,
}

impl Signal {
    fn clear(&mut self, window: &mut Window) {
        for image in std::mem::take(&mut self.images).into_values() {
            window
                .drop_image(image)
                .expect("failed to release spectrum");
        }
    }
}

pub struct Visual {
    pub mode: ViewMode,
    pub span: f64,
    pub delay: f64,
    pub bands: usize,
    pub position: usize,
    signal: Option<Signal>,
}

impl Visual {
    pub fn new() -> Self {
        Self {
            mode: ViewMode::Combined,
            span: 96.0,
            delay: 0.0,
            bands: 128,
            position: 0,
            signal: None,
        }
    }

    pub fn set_track(&mut self, track: Arc<Track>, window: &mut Window) {
        if let Some(signal) = &mut self.signal {
            signal.clear(window);
        }
        self.position = 0;
        self.signal = Some(Signal {
            analyzer: Analyzer::new(track.sample_rate, self.bands),
            track,
            images: BTreeMap::new(),
            format: None,
        });
    }

    pub fn set_bands(&mut self, bands: usize) {
        self.bands = bands;
        if let Some(signal) = &mut self.signal {
            signal.analyzer = Analyzer::new(signal.track.sample_rate, bands);
            signal.format = None;
        }
    }

    fn spectrum(
        &mut self,
        view: Viewport,
        colors: [[u8; 3]; 2],
        window: &mut Window,
    ) -> Vec<(i64, Arc<RenderImage>)> {
        let Some(signal) = &mut self.signal else {
            return Vec::new();
        };
        let format = Format {
            width: view.width,
            height: view.height,
            samples: view.samples,
            colors,
        };
        if signal.format.as_ref() != Some(&format) {
            signal.clear(window);
            signal.format = Some(format);
        }
        let first = (view.end_bin() - view.width as i64).div_euclid(TILE_WIDTH);
        let last = (view.end_bin() - 1).div_euclid(TILE_WIDTH);
        let visible = first..=last;
        for (_, image) in signal
            .images
            .extract_if(.., |tile, _| !visible.contains(tile))
        {
            window
                .drop_image(image)
                .expect("failed to release spectrum");
        }
        for tile in visible {
            if let Entry::Vacant(slot) = signal.images.entry(tile) {
                // Adjacent columns keep texture filtering continuous across tile edges.
                let columns = tile * TILE_WIDTH - 1..(tile + 1) * TILE_WIDTH + 1;
                let pixels = signal.analyzer.render(&signal.track, view, columns, colors);
                let buffer =
                    RgbaImage::from_raw((TILE_WIDTH + 2) as u32, view.height as u32, pixels)
                        .expect("spectrum dimensions match buffer");
                slot.insert(Arc::new(RenderImage::new([Frame::new(buffer)])));
            }
        }
        signal
            .images
            .iter()
            .map(|(&tile, image)| (tile, image.clone()))
            .collect()
    }
}

impl Render for Visual {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(signal) = &self.signal else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(IconName::AudioLines)
                        .size(px(48.0))
                        .text_color(cx.theme().muted_foreground),
                )
                .into_any_element();
        };
        let ink: Hsla = if cx.theme().is_dark() {
            rgb(0x80c9f3)
        } else {
            rgb(0x1674bb)
        }
        .into();
        let background = cx.theme().background;
        let colors = [background, ink].map(|color| {
            let color = color.to_rgb();
            [color.r, color.g, color.b].map(|channel| (channel * 255.0).round() as u8)
        });
        let track = signal.track.clone();
        let samples = (f64::from(track.sample_rate) * self.span / 1000.0).max(2.0);
        let position = (self.position as f64 - f64::from(track.sample_rate) * self.delay / 1000.0)
            .clamp(-samples, track.frames() as f64 + samples);
        let spectrum = cx.entity();
        let panel = || {
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .rounded_lg()
                .overflow_hidden()
                .border_1()
                .border_color(cx.theme().border)
                .bg(background)
        };
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .when(self.mode != ViewMode::Waveform, |this| {
                this.child(
                    panel().child(
                        canvas(
                            move |bounds, window, cx| {
                                let (bounds, view) = viewport(position, samples, bounds, window);
                                let images = spectrum.update(cx, |spectrum, _| {
                                    spectrum.spectrum(view, colors, window)
                                });
                                (bounds, view, images)
                            },
                            move |_, (bounds, view, images), window, _| {
                                let pixel = bounds.size.width / view.width as f32;
                                let first = view.end_bin() - view.width as i64;
                                for (tile, image) in images {
                                    let left =
                                        bounds.left() + pixel * (tile * TILE_WIDTH - first) as f32;
                                    let core = Bounds::new(
                                        point(left, bounds.top()),
                                        size(pixel * TILE_WIDTH as f32, bounds.size.height),
                                    );
                                    let image_bounds = Bounds::new(
                                        point(left - pixel, bounds.top()),
                                        size(pixel * (TILE_WIDTH + 2) as f32, bounds.size.height),
                                    );
                                    window
                                        .paint_image(
                                            core.intersect(&bounds),
                                            image_bounds,
                                            Default::default(),
                                            image,
                                            0,
                                            false,
                                        )
                                        .expect("failed to paint spectrum");
                                }
                                playhead(view, bounds, ink, window);
                            },
                        )
                        .size_full(),
                    ),
                )
            })
            .when(self.mode != ViewMode::Spectrum, |this| {
                this.child(
                    panel().child(
                        canvas(
                            move |bounds, window, _| viewport(position, samples, bounds, window),
                            move |_, (bounds, view), window, _| {
                                waveform(&track, view, bounds, ink, window);
                                playhead(view, bounds, ink, window);
                            },
                        )
                        .size_full(),
                    ),
                )
            })
            .into_any_element()
    }
}

fn viewport(
    position: f64,
    samples: f64,
    bounds: Bounds<Pixels>,
    window: &Window,
) -> (Bounds<Pixels>, Viewport) {
    let bounds = Bounds::from_corners(
        window.pixel_snap_point(bounds.origin),
        window.pixel_snap_point(bounds.bottom_right()),
    );
    let scale = window.scale_factor();
    let view = Viewport {
        position,
        samples,
        width: (f32::from(bounds.size.width) * scale).round().max(1.0) as usize,
        height: (f32::from(bounds.size.height) * scale).round().max(1.0) as usize,
    };
    (bounds, view)
}

fn playhead(view: Viewport, bounds: Bounds<Pixels>, ink: Hsla, window: &mut Window) {
    let x = (view.position - view.start()) / view.samples;
    window.paint_quad(fill(
        Bounds::new(
            point(bounds.left() + bounds.size.width * x as f32, bounds.top()),
            size(px(1.0 / window.scale_factor()), bounds.size.height),
        ),
        ink.opacity(0.18),
    ));
}

fn waveform(track: &Track, view: Viewport, bounds: Bounds<Pixels>, ink: Hsla, window: &mut Window) {
    let first = view.start();
    let center = bounds.center().y;
    let amplitude = bounds.size.height * 0.46;
    let pixel = px(1.0 / window.scale_factor());
    let mut path = PathBuilder::stroke(px(1.25));
    if view.samples <= view.width as f64 {
        let start = first.floor() as i64;
        let end = (first + view.samples).ceil() as i64;
        for frame in start..=end {
            let sample = if frame < 0 || frame as usize >= track.frames() {
                0.0
            } else {
                track.sample(frame as usize)
            };
            let x =
                bounds.left() + bounds.size.width * ((frame as f64 - first) / view.samples) as f32;
            let position = point(x, center - amplitude * sample);
            if frame == start {
                path.move_to(position);
            } else {
                path.line_to(position);
            }
        }
    } else {
        for x in 0..view.width {
            let start = view.sample_at(x as f64).floor().max(0.0) as usize;
            let end =
                (view.sample_at((x + 1) as f64).floor().max(0.0) as usize).min(track.frames());
            let (low, high) = if start < end {
                (start..end)
                    .map(|frame| track.sample(frame))
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| {
                        (low.min(value), high.max(value))
                    })
            } else {
                (0.0, 0.0)
            };
            let top = center - amplitude * high;
            let bottom = center - amplitude * low;
            let left = bounds.left() + pixel * x as f32;
            window.paint_quad(fill(
                Bounds::new(point(left, top), size(pixel, (bottom - top).max(pixel))),
                ink,
            ));
            let position = point(left + pixel * 0.5, (top + bottom) * 0.5);
            if x == 0 {
                path.move_to(position);
            } else {
                path.line_to(position);
            }
        }
    }
    window.paint_path(path.build().expect("finite waveform points"), ink);
}
