use std::sync::{Arc, mpsc};

use anyhow::{Context, Result};
use cpal::{
    Stream, StreamConfig, StreamInstant,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use triple_buffer::{Input, Output, triple_buffer};

use crate::audio::Track;

#[derive(Clone, Copy)]
struct Controls {
    playing: bool,
    repeat: bool,
    volume: f32,
    seek: (u64, usize),
}

#[derive(Clone, Copy)]
struct Clock {
    frame: usize,
    at: StreamInstant,
    playing: bool,
    repeat: bool,
    seek: u64,
}

pub struct Playback {
    pub track: Arc<Track>,
    stream: Stream,
    controls: Controls,
    input: Input<Controls>,
    clock: Output<Clock>,
    errors: mpsc::Receiver<cpal::Error>,
}

impl Playback {
    pub fn new(track: Arc<Track>, volume: f32, repeat: bool) -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .context("没有可用的音频输出设备")?;
        let config = StreamConfig {
            channels: track.channels.try_into()?,
            sample_rate: track.sample_rate,
            buffer_size: Default::default(),
        };
        let controls = Controls {
            playing: false,
            repeat,
            volume,
            seek: (0, 0),
        };
        let (input, mut output) = triple_buffer(&controls);
        let (mut clock_input, clock) = triple_buffer(&Clock {
            frame: 0,
            at: StreamInstant::ZERO,
            playing: false,
            repeat,
            seek: 0,
        });
        let (error_tx, errors) = mpsc::channel();
        let audio = track.clone();
        let mut cursor = 0;
        let mut seek = 0;
        let mut repeating = repeat;
        let stream = device
            .build_output_stream(
                config,
                move |buffer: &mut [f32], info| {
                    let controls = *output.read();
                    if repeating && !controls.repeat {
                        cursor %= audio.frames();
                    }
                    repeating = controls.repeat;
                    if controls.seek.0 != seek {
                        seek = controls.seek.0;
                        cursor = controls.seek.1;
                    }
                    let frame = cursor;
                    for output in buffer.chunks_exact_mut(audio.channels) {
                        if controls.playing && (controls.repeat || cursor < audio.frames()) {
                            let offset = (cursor % audio.frames()) * audio.channels;
                            for (output, sample) in output.iter_mut().zip(&audio.samples[offset..])
                            {
                                *output = sample * controls.volume;
                            }
                        } else {
                            output.fill(0.0);
                        }
                        if controls.playing {
                            cursor += 1;
                        }
                    }
                    clock_input.write(Clock {
                        frame,
                        at: info.timestamp().playback,
                        playing: controls.playing,
                        repeat: controls.repeat,
                        seek,
                    });
                },
                move |error| {
                    let _ = error_tx.send(error);
                },
                None,
            )
            .context("无法打开音频输出")?;
        stream.play().context("无法开始播放")?;
        Ok(Self {
            track,
            stream,
            controls,
            input,
            clock,
            errors,
        })
    }

    pub fn position(&mut self) -> usize {
        let clock = *self.clock.read();
        if clock.seek != self.controls.seek.0 {
            return self.controls.seek.1;
        }
        if !clock.playing {
            return clock.frame;
        }
        let elapsed = (self.stream.now().as_nanos() as f64 - clock.at.as_nanos() as f64) * 1e-9;
        let frame =
            (clock.frame as f64 + elapsed * f64::from(self.track.sample_rate)).max(0.0) as usize;
        if clock.repeat {
            frame % self.track.frames()
        } else {
            frame.min(self.track.frames())
        }
    }

    pub fn is_playing(&self) -> bool {
        self.controls.playing
    }

    pub fn seek(&mut self, frame: usize) {
        self.controls.seek = (self.controls.seek.0 + 1, frame.min(self.track.frames()));
        self.input.write(self.controls);
    }

    pub fn toggle(&mut self) {
        let position = self.position();
        self.controls.playing = !self.controls.playing;
        self.seek(if position == self.track.frames() {
            0
        } else {
            position
        });
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.controls.volume = volume;
        self.input.write(self.controls);
    }

    pub fn set_repeat(&mut self, repeat: bool) {
        self.controls.repeat = repeat;
        self.input.write(self.controls);
    }

    pub fn update(&mut self) -> Result<usize> {
        if let Ok(error) = self.errors.try_recv() {
            self.controls.playing = false;
            self.input.write(self.controls);
            return Err(error).context("音频输出中断");
        }
        let position = self.position();
        if position == self.track.frames() && self.controls.playing {
            self.controls.playing = false;
            self.seek(position);
        }
        Ok(position)
    }
}
