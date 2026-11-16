use std::{path::PathBuf, sync::Arc};

use gpui::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt, Theme, TitleBar,
    button::{Button, ButtonVariants},
    slider::{Slider, SliderEvent, SliderState},
};
use gpui::{
    App, Context, Entity, ExternalPaths, FocusHandle, KeyBinding, PathPromptOptions, Render, Task,
    Window, actions, assets::IconName, div, prelude::*, px,
};

use crate::{audio::Track, playback::Playback};

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
        cx.subscribe_in(&seek, window, |this, _, event, _, cx| {
            match event {
                SliderEvent::Change(value) => {
                    this.seeking = true;
                    if let Some(playback) = &mut this.playback {
                        this.position = (f64::from(value.end()) * playback.track.frames() as f64)
                            .round() as usize;
                        playback.seek(this.position);
                    }
                }
                SliderEvent::Release(_) => this.seeking = false,
            }
            cx.notify();
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
        let details =
            track.map(|track| format!("{} Hz  /  {} 声道", track.sample_rate, track.channels));
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
                    .child(div().flex_1().min_w_0().truncate().child(name.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .px_8()
                    .child(
                        Icon::new(IconName::AudioLines)
                            .size(px(48.0))
                            .text_color(cx.theme().primary),
                    )
                    .child(div().text_xl().font_semibold().child(name))
                    .when_some(details, |this, details| {
                        this.child(div().text_color(cx.theme().muted_foreground).child(details))
                    }),
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
