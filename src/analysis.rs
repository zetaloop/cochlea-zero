use std::{f64::consts::TAU, ops::Range};

use rayon::prelude::*;

use crate::audio::Track;

#[derive(Clone, Copy)]
pub struct Viewport {
    pub position: f64,
    pub samples: f64,
    pub width: usize,
    pub height: usize,
}

impl Viewport {
    pub fn end_bin(self) -> i64 {
        ((self.position + self.samples * 0.5) * self.width as f64 / self.samples).floor() as i64
    }

    pub fn start(self) -> f64 {
        self.sample_at(0.0)
    }

    pub fn sample_at(self, column: f64) -> f64 {
        ((self.end_bin() - self.width as i64) as f64 + column) * self.samples / self.width as f64
    }
}

struct Band {
    b0: f64,
    a1: f64,
    a2: f64,
    state: [f64; 4],
    warmup: usize,
    history: Vec<f32>,
}

impl Band {
    fn new(rate: u32, position: f64) -> Self {
        let frequency = 30.0 * ((f64::from(rate) * 0.42).min(16000.0) / 30.0).powf(position);
        let omega = TAU * frequency / f64::from(rate);
        let alpha = omega.sin() / (2.0 * (3.8 + 3.2 * position));
        let a2 = (1.0 - alpha) / (1.0 + alpha);
        Self {
            b0: alpha / (1.0 + alpha),
            a1: -2.0 * omega.cos() / (1.0 + alpha),
            a2,
            state: [0.0; 4],
            // Earlier filter state decays below double-precision resolution.
            warmup: (f64::EPSILON.ln() / a2.sqrt().ln()).ceil() as usize,
            history: Vec::new(),
        }
    }

    fn step(&mut self, sample: f32) -> f32 {
        let x = f64::from(sample);
        let [x1, x2, y1, y2] = self.state;
        let y = self.b0 * x - self.b0 * x2 - self.a1 * y1 - self.a2 * y2;
        self.state = [x, x1, y, y1];
        y as f32
    }

    fn sample(&self, frame: i64, frames: usize) -> f32 {
        if frame < 0 || frame as usize >= frames {
            0.0
        } else {
            self.history[frame as usize % self.history.len()]
        }
    }
}

pub struct Analyzer {
    bands: Vec<Band>,
    start: usize,
    end: usize,
    capacity: usize,
    levels: Vec<f32>,
    colors: Option<[[u8; 3]; 2]>,
    palette: Vec<[u8; 4]>,
}

impl Analyzer {
    pub fn new(rate: u32, bands: usize) -> Self {
        Self {
            bands: (0..bands)
                .map(|band| Band::new(rate, band as f64 / (bands - 1) as f64))
                .collect(),
            start: 0,
            end: 0,
            capacity: 0,
            levels: Vec::new(),
            colors: None,
            palette: Vec::new(),
        }
    }

    pub fn render(
        &mut self,
        track: &Track,
        view: Viewport,
        columns: Range<i64>,
        colors: [[u8; 3]; 2],
    ) -> Vec<u8> {
        let width = (columns.end - columns.start) as usize;
        let offset = (columns.start - view.end_bin() + view.width as i64) as f64;
        if self.colors != Some(colors) {
            self.palette = (0..4096)
                .map(|index| {
                    let light = (1.0 - (-f64::from(index) / 384.0).exp()).powf(0.64);
                    let color = std::array::from_fn::<_, 3, _>(|channel| {
                        (f64::from(colors[0][channel])
                            + (f64::from(colors[1][channel]) - f64::from(colors[0][channel]))
                                * light)
                            .round() as u8
                    });
                    [color[2], color[1], color[0], 255]
                })
                .collect();
            self.colors = Some(colors);
        }
        self.levels.resize(width * self.bands.len(), 0.0);

        let start = view.sample_at(offset).floor().max(0.0) as usize;
        let end = (view.sample_at(offset + width as f64).ceil().max(0.0) as usize + 1)
            .min(track.frames());
        let capacity = (width as f64 * view.samples / view.width as f64).ceil() as usize + 4;
        let reset = self.capacity != capacity
            || start < self.start
            || start.saturating_sub(self.end) > self.bands[0].warmup;
        let start = start.min(track.frames());
        if reset || end > self.end {
            self.bands.par_iter_mut().for_each(|band| {
                let from = if reset {
                    band.state.fill(0.0);
                    band.history.resize(capacity, 0.0);
                    start.saturating_sub(band.warmup)
                } else {
                    self.end
                };
                for frame in from..start {
                    band.step(track.sample(frame));
                }
                for frame in from.max(start)..end {
                    let value = band.step(track.sample(frame));
                    band.history[frame % capacity] = value;
                }
            });
            self.start = if reset {
                start
            } else {
                self.start.max(end.saturating_sub(capacity))
            };
            self.end = end;
            self.capacity = capacity;
        }

        let step = view.samples / view.width as f64;
        self.bands
            .par_iter()
            .zip(self.levels.par_chunks_exact_mut(width))
            .for_each(|(band, levels)| {
                for (x, level) in levels.iter_mut().enumerate() {
                    if step >= 1.0 {
                        let from = view.sample_at(offset + x as f64).floor().max(0.0) as usize;
                        let to = (view.sample_at(offset + (x + 1) as f64).floor().max(0.0)
                            as usize)
                            .min(track.frames());
                        *level = (from..to)
                            .map(|frame| strength(band.history[frame % capacity]))
                            .fold(0.0, f32::max);
                    } else {
                        let index = view.sample_at(offset + x as f64 + 0.5);
                        let frame = index.floor() as i64;
                        let fraction = (index - frame as f64) as f32;
                        let a = band.sample(frame, track.frames());
                        let b = band.sample(frame + 1, track.frames());
                        *level = strength(a + (b - a) * fraction);
                    }
                }
            });
        let mut pixels = vec![0; width * view.height * 4];
        pixels
            .par_chunks_exact_mut(width * 4)
            .enumerate()
            .for_each(|(y, row)| {
                let position = (1.0 - y as f64 / (view.height - 1).max(1) as f64)
                    * (self.bands.len() - 1) as f64;
                let lower = (position.floor() as usize).min(self.bands.len() - 2);
                let fraction = (position - lower as f64) as f32;
                let gain = (12.5 * (1.0 + position / self.bands.len() as f64 * 2.3) * 384.0) as f32;
                let a = &self.levels[lower * width..];
                let b = &self.levels[(lower + 1) * width..];
                for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let level = a[x] * (1.0 - fraction) + b[x] * fraction;
                    *pixel = self.palette[((level * gain) as usize).min(4095)];
                }
            });
        pixels
    }
}

fn strength(value: f32) -> f32 {
    if value >= 0.0 { value } else { -value * 0.42 }
}
