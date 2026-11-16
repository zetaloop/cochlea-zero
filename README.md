# Cochlea Zero

基于 GPUI 的音频播放器，提供频谱纹理、波形及上下排列视图，支持时间跨度和频段数量调节。

## 运行

需要 Rust，以及位于 PATH 中的 `ffmpeg` 和 `ffprobe`。

```sh
cargo run --release
```

打开或拖入音频文件即可播放。空格切换播放和暂停，`Ctrl+O` 打开文件；macOS 对应 `Cmd+O`。

## 构建

```sh
cargo build --release
```

生成的程序位于 `target/release/`。
