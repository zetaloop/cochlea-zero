use std::sync::Arc;

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

struct Signal {
    track: Arc<Track>,
    analyzer: Analyzer,
    image: Option<Arc<RenderImage>>,
}

pub struct Visual {
    pub mode: ViewMode,
    pub span: f64,
    pub bands: usize,
    pub position: usize,
    signal: Option<Signal>,
}

impl Visual {
    pub fn new() -> Self {
        Self {
            mode: ViewMode::Combined,
            span: 96.0,
            bands: 128,
            position: 0,
            signal: None,
        }
    }

    pub fn set_track(&mut self, track: Arc<Track>, window: &mut Window) {
        if let Some(signal) = self.signal.take()
            && let Some(image) = signal.image
        {
            window
                .drop_image(image)
                .expect("failed to release spectrum");
        }
        self.position = 0;
        self.signal = Some(Signal {
            analyzer: Analyzer::new(track.sample_rate, self.bands),
            track,
            image: None,
        });
    }

    pub fn set_bands(&mut self, bands: usize) {
        self.bands = bands;
        if let Some(signal) = &mut self.signal {
            signal.analyzer = Analyzer::new(signal.track.sample_rate, bands);
        }
    }

    fn spectrum(
        &mut self,
        view: Viewport,
        colors: [[u8; 3]; 2],
        window: &mut Window,
    ) -> Option<Arc<RenderImage>> {
        let signal = self.signal.as_mut()?;
        if let Some(pixels) = signal.analyzer.render(&signal.track, view, colors) {
            let buffer =
                RgbaImage::from_raw(view.width as u32, view.height as u32, pixels.to_vec())
                    .expect("spectrum dimensions match buffer");
            let image = Arc::new(RenderImage::new([Frame::new(buffer)]));
            if let Some(previous) = signal.image.replace(image) {
                window
                    .drop_image(previous)
                    .expect("failed to release spectrum");
            }
        }
        signal.image.clone()
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
        let position = self.position;
        let samples = (f64::from(track.sample_rate) * self.span / 1000.0).max(2.0);
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
                                let view = viewport(position, samples, bounds, window);
                                let image = spectrum.update(cx, |spectrum, _| {
                                    spectrum.spectrum(view, colors, window)
                                });
                                (view, image)
                            },
                            move |bounds, (view, image), window, _| {
                                if let Some(image) = image {
                                    window
                                        .paint_image(
                                            bounds,
                                            bounds,
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
                            move |bounds, view, window, _| {
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

fn viewport(position: usize, samples: f64, bounds: Bounds<Pixels>, window: &Window) -> Viewport {
    let scale = window.scale_factor();
    Viewport {
        position,
        samples,
        width: (f32::from(bounds.size.width) * scale).round().max(1.0) as usize,
        height: (f32::from(bounds.size.height) * scale).round().max(1.0) as usize,
    }
}

fn playhead(view: Viewport, bounds: Bounds<Pixels>, ink: Hsla, window: &mut Window) {
    let x = (view.position as f64 - view.start()) / view.samples;
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
