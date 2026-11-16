use std::{path::Path, process::Command};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

pub struct Track {
    pub name: String,
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl Track {
    pub fn open(path: &Path) -> Result<Self> {
        let probe = command("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=sample_rate,channels",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .context("无法启动 ffprobe")?;
        ensure!(
            probe.status.success(),
            "读取音频信息失败：{}",
            String::from_utf8_lossy(&probe.stderr).trim()
        );
        let metadata: Value = serde_json::from_slice(&probe.stdout)?;
        let stream = &metadata["streams"][0];
        let sample_rate: u32 = stream["sample_rate"]
            .as_str()
            .context("音轨缺少采样率")?
            .parse()?;
        let channels = usize::try_from(stream["channels"].as_u64().context("音轨缺少声道信息")?)?;
        ensure!(sample_rate > 0 && channels > 0, "音频格式无效");
        let decoded = command("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:a:0", "-f", "f32le", "pipe:1"])
            .output()
            .context("无法启动 ffmpeg")?;
        ensure!(
            decoded.status.success(),
            "解码音频失败：{}",
            String::from_utf8_lossy(&decoded.stderr).trim()
        );
        ensure!(
            !decoded.stdout.is_empty() && decoded.stdout.len().is_multiple_of(4 * channels),
            "解码后的音频数据不完整"
        );
        let samples = decoded
            .stdout
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&bytes| f32::from_le_bytes(bytes))
            .collect();
        Ok(Self {
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            sample_rate,
            channels,
            samples,
        })
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / f64::from(self.sample_rate)
    }
}

fn command(program: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(program);
        command.creation_flags(0x0800_0000);
        command
    }
    #[cfg(not(windows))]
    Command::new(program)
}
