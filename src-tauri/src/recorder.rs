/* ---------- 麦克风采集：cpal 输入流 → i16 累积 → WAV(hound) → base64 ---------- */
/* 替代原 HTML 前端的 getUserMedia+WAV 编码；电平表供跑马灯呼吸强度。 */

use base64::Engine;
use std::sync::{Arc, Mutex};

pub struct Recorder {
    samples: Arc<Mutex<Vec<i16>>>,
    sample_rate: u32,
    channels: u16,
    started: std::time::Instant,
    // Stream drop 即停流；保字段持有
    _stream: cpal::Stream,
}

pub fn start() -> Result<Recorder, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let device = cpal::default_host()
        .default_input_device()
        .ok_or("没有找到麦克风设备")?;
    let cfg = device
        .default_input_config()
        .map_err(|e| format!("麦克风配置读取失败: {e}"))?;
    let rate = cfg.sample_rate();
    let channels = cfg.channels();
    let samples: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::with_capacity(
        rate as usize * channels as usize * 30,
    )));

    let sink = samples.clone();
    let fmt = cfg.sample_format();
    let stream_cfg: cpal::StreamConfig = cfg.into();
    let stream = match fmt {
        cpal::SampleFormat::I16 => device.build_input_stream(
            stream_cfg.clone(),
            move |d: &[i16], _| {
                if let Ok(mut s) = sink.lock() {
                    s.extend_from_slice(d);
                }
            },
            |e| eprintln!("vcc: mic err {e}"),
            None,
        ),
        cpal::SampleFormat::F32 => device.build_input_stream(
            stream_cfg.clone(),
            move |d: &[f32], _| {
                if let Ok(mut s) = sink.lock() {
                    s.extend(d.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16));
                }
            },
            |e| eprintln!("vcc: mic err {e}"),
            None,
        ),
        f => return Err(format!("不支持的麦克风采样格式 {f:?}")),
    }
    .map_err(|e| format!("麦克风流启动失败: {e}"))?;
    stream.play().map_err(|e| format!("麦克风启动失败: {e}"))?;

    Ok(Recorder {
        samples,
        sample_rate: rate,
        channels,
        started: std::time::Instant::now(),
        _stream: stream,
    })
}

impl Recorder {
    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }

    /// 尾部 1/4 秒 RMS 电平（0..1+），供跑马灯呼吸；只读不清空（stop 时还要全量编码）
    pub fn level(&self) -> f32 {
        let s = match self.samples.lock() {
            Ok(s) => s,
            Err(_) => return 0.0,
        };
        let per_quarter = self.sample_rate as usize * self.channels as usize / 4;
        let n = s.len().min(per_quarter);
        if n == 0 {
            return 0.0;
        }
        let sum: f64 = s[s.len() - n..]
            .iter()
            .map(|x| {
                let v = *x as f64 / 32768.0;
                v * v
            })
            .sum();
        (sum / n as f64).sqrt() as f32
    }

    /// 停止录音并编码 WAV base64（消耗 self：Stream drop 停流）。
    /// WAV 头手写（PCM 16bit）——hound 3.5 的 WavWriter 没有 into_inner，取不出字节。
    pub fn stop(self) -> Result<String, String> {
        let samples = match self.samples.lock() {
            Ok(s) => s.clone(),
            Err(_) => Vec::new(),
        };
        if samples.len() < self.sample_rate as usize / 5 {
            return Err("录音太短".into());
        }
        let bytes = wav_bytes(&samples, self.sample_rate, self.channels);
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }
}

/// 标准 PCM WAV（44 字节头 + LE i16 数据）
fn wav_bytes(samples: &[i16], rate: u32, channels: u16) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes()); // fmt 块长
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes()); // byte rate
    b.extend_from_slice(&(channels * 2).to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}
