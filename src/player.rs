use std::{path::PathBuf, sync::Arc};

use gpui::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, Theme, TitleBar,
    button::{Button, ButtonGroup, ButtonVariants},
    slider::{Slider, SliderEvent, SliderScale, SliderState},
};
use gpui::{
    App, Context, Entity, ExternalPaths, FocusHandle, KeyBinding, PathPromptOptions, Render, Task,
    Window, actions, assets::IconName, div, prelude::*, px,
};

use crate::{
    audio::Track,
    playback::Playback,
    visual::{ViewMode, Visual},
};

actions!(cochlea, [Open, TogglePlayback]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("space", TogglePlayback, Some("Player")),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-o"
            } else {
                "ctrl-o"
            },
            Open,
            Some("Player"),
        ),
    ]);
}

pub struct Player {
    focus: FocusHandle,
    playback: Option<Playback>,
    loading: Option<Task<()>>,
    seek: Entity<SliderState>,
    volume: Entity<SliderState>,
    span: Entity<SliderState>,
    visual: Entity<Visual>,
    repeat: bool,
    seeking: bool,
    position: usize,
    error: Option<String>,
}

impl Player {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let seek = cx.new(|_| SliderState::new().max(1.0).step(0.0));
        let volume = cx.new(|_| SliderState::new().max(1.0).step(0.01).default_value(1.0));
        let span = cx.new(|_| {
            SliderState::new()
                .min(2.0)
                .max(500.0)
                .step(0.1)
                .scale(SliderScale::Logarithmic)
                .default_value(96.0)
        });
        let visual = cx.new(|_| Visual::new());
        cx.subscribe_in(&seek, window, |this, _, event, _, cx| {
            match event {
                SliderEvent::Change(value) => {
                    this.seeking = true;
                    if let Some(playback) = &mut this.playback {
                        this.position = (f64::from(value.end()) * playback.track.frames() as f64)
                            .round() as usize;
                        playback.seek(this.position);
                        this.visual.update(cx, |visual, cx| {
                            visual.position = this.position;
                            cx.notify();
                        });
                    }
                }
                SliderEvent::Release(_) => this.seeking = false,
            }
            cx.notify();
        })
        .detach();
        cx.subscribe(&span, |this, _, event, cx| {
            if let SliderEvent::Change(value) = event {
                this.visual.update(cx, |visual, cx| {
                    visual.span = f64::from(value.end());
                    cx.notify();
                });
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&volume, |this, _, event, _| {
            if let SliderEvent::Change(value) = event
                && let Some(playback) = &mut this.playback
            {
                playback.set_volume(value.end());
            }
        })
        .detach();
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
        })
        .detach();
        Self {
            focus,
            playback: None,
            loading: None,
            seek,
            volume,
            span,
            visual,
            repeat: false,
            seeking: false,
            position: 0,
            error: None,
        }
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("打开音频".into()),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.load(path, window, cx);
                    });
                }
            }
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("打开文件失败：{error}"));
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn load(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        let volume = self.volume.read(cx).value().end();
        let repeat = self.repeat;
        let work = cx.background_executor().spawn(async move {
            let track = Arc::new(Track::open(&path)?);
            Playback::new(track, volume, repeat)
        });
        self.loading = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(mut playback) => {
                        playback.set_volume(this.volume.read(cx).value().end());
                        playback.set_repeat(this.repeat);
                        this.visual.update(cx, |visual, cx| {
                            visual.set_track(playback.track.clone(), window);
                            cx.notify();
                        });
                        this.playback = Some(playback);
                        if let Some(playback) = &mut this.playback {
                            playback.toggle();
                        }
                        this.position = 0;
                        this.seek
                            .update(cx, |seek, cx| seek.set_value(0.0, window, cx));
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                this.loading = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn toggle(&mut self, _: &TogglePlayback, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(playback) = &mut self.playback {
            playback.toggle();
            self.tick(window, cx);
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(playback) = &mut self.playback {
            match playback.update() {
                Ok(position) if !self.seeking => {
                    self.position = position;
                    let value = position as f32 / playback.track.frames() as f32;
                    self.seek
                        .update(cx, |seek, cx| seek.set_value(value, window, cx));
                    self.visual.update(cx, |visual, cx| {
                        visual.position = position;
                        cx.notify();
                    });
                }
                Err(error) => self.error = Some(format!("{error:#}")),
                _ => {}
            }
        }
        cx.notify();
    }
}

impl Render for Player {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let playing = self.playback.as_ref().is_some_and(Playback::is_playing);
        if playing {
            let this = cx.weak_entity();
            window.on_next_frame(move |window, cx| {
                let _ = this.update(cx, |this, cx| this.tick(window, cx));
            });
        }
        let track = self.playback.as_ref().map(|playback| &playback.track);
        let name = track
            .map_or("Cochlea Zero", |track| track.name.as_str())
            .to_owned();
        let time = track.map_or("0:00 / 0:00".to_owned(), |track| {
            format!(
                "{} / {}",
                time(self.position as f64 / f64::from(track.sample_rate)),
                time(track.duration())
            )
        });
        let modes = [
            (ViewMode::Spectrum, "频谱"),
            (ViewMode::Waveform, "波形"),
            (ViewMode::Combined, "上下排列"),
        ];
        let mode = self.visual.read(cx).mode;
        let bands = self.visual.read(cx).bands;
        let span = (self.span.read(cx).value().end() * 10.0).round() / 10.0;
        div()
            .id("player")
            .key_context("Player")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::toggle))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(path) = paths.paths().first() {
                    this.load(path.clone(), window, cx);
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .text_sm()
            .child(
                TitleBar::new().child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Cochlea Zero"),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("open")
                            .icon(IconName::FolderOpen)
                            .label("打开音频")
                            .loading(self.loading.is_some())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(&Open, window, cx);
                            })),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(name)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .px_5()
                    .pt_4()
                    .child(
                        ButtonGroup::new("view")
                            .small()
                            .children(modes.map(|(value, label)| {
                                Button::new(label).label(label).selected(mode == value)
                            }))
                            .on_click(cx.listener(move |this, indices: &Vec<usize>, _, cx| {
                                if let Some(&index) = indices.first() {
                                    this.visual.update(cx, |visual, cx| {
                                        visual.mode = modes[index].0;
                                        cx.notify();
                                    });
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w(px(240.0))
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("时间跨度"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .child(Slider::new(&self.span).accessibility_label("时间跨度")),
                            )
                            .child(div().w(px(68.0)).text_right().child(format!("{span} ms"))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_color(cx.theme().muted_foreground).child("频段"))
                            .child(
                                ButtonGroup::new("bands")
                                    .small()
                                    .children([128, 256, 512].map(|value| {
                                        Button::new(("bands", value))
                                            .label(value.to_string())
                                            .selected(bands == value)
                                    }))
                                    .on_click(cx.listener(|this, indices: &Vec<usize>, _, cx| {
                                        if let Some(&index) = indices.first() {
                                            this.visual.update(cx, |visual, cx| {
                                                visual.set_bands([128, 256, 512][index]);
                                                cx.notify();
                                            });
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_5()
                    .py_4()
                    .child(self.visual.clone()),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_5()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Slider::new(&self.seek)
                            .disabled(track.is_none())
                            .accessibility_label("播放进度"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("play")
                                    .primary()
                                    .icon(if playing {
                                        IconName::Pause
                                    } else {
                                        IconName::Play
                                    })
                                    .tooltip(if playing { "暂停" } else { "播放" })
                                    .disabled(track.is_none())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle(&TogglePlayback, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("repeat")
                                    .ghost()
                                    .icon(IconName::Repeat1)
                                    .selected(self.repeat)
                                    .tooltip("单曲循环")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.repeat = !this.repeat;
                                        if let Some(playback) = &mut this.playback {
                                            playback.set_repeat(this.repeat);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(div().text_color(cx.theme().muted_foreground).child(time))
                            .child(div().flex_1())
                            .child(
                                Icon::new(IconName::Volume2)
                                    .small()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child(
                                div()
                                    .w(px(104.0))
                                    .child(Slider::new(&self.volume).accessibility_label("音量")),
                            ),
                    ),
            )
    }
}

fn time(seconds: f64) -> String {
    let seconds = seconds as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
