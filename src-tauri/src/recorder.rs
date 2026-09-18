/* ---------- 麦克风采集：cpal 输入流 → i16 累积 → 下混单声道+重采样 16k → WAV → base64 ---------- */
/* whisper 契约：16kHz / 单声道 / 16bit PCM WAV（voice.rs validate_wav 强校验）。
   设备原始格式（常见 48k 立体声）在 stop 时统一转换，采集回调保持零转换低开销。 */

use base64::Engine;
use std::sync::atomic::{AtomicBool, Ordering};
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

    // WASAPI 偶发 "A buffer underrun or overrun occurred" 属瞬时抖动（Chrome 同样忽略）；
    // 其余错误只报第一次，避免刷屏
    let quiet = Arc::new(AtomicBool::new(false));
    // 闭包工厂：I16/F32 两个分支各 clone 一份（Arc 非 Copy）
    let err_cb = || {
        let quiet = quiet.clone();
        move |e: cpal::Error| {
            let s = e.to_string();
            if s.contains("underrun") || s.contains("overrun") {
                return; // WASAPI 瞬时缓冲抖动，流仍正常，忽略
            }
            if !quiet.swap(true, Ordering::Relaxed) {
                eprintln!("vcc: mic err {s}");
            }
        }
    };

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
            err_cb(),
            None,
        ),
        cpal::SampleFormat::F32 => device.build_input_stream(
            stream_cfg.clone(),
            move |d: &[f32], _| {
                if let Ok(mut s) = sink.lock() {
                    s.extend(d.iter().map(|x| (x.clamp(-1.0, 1.0) * 32767.0) as i16));
                }
            },
            err_cb(),
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
    /// 输出恒为 16kHz/单声道/16bit——whisper 契约，与设备原始格式无关。
    pub fn stop(self) -> Result<String, String> {
        let samples = match self.samples.lock() {
            Ok(s) => s.clone(),
            Err(_) => Vec::new(),
        };
        if samples.len() < self.sample_rate as usize / 5 {
            return Err("录音太短".into());
        }
        let bytes = encode_16k_mono_wav(&samples, self.sample_rate, self.channels);
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }
}

/// 下混单声道 + 线性插值重采样到 16kHz + 44 字节头 PCM WAV（16bit LE）。
/// pub 供测试：头字段（声道=1 / 率=16000）必须严格满足 whisper 校验。
pub fn encode_16k_mono_wav(samples: &[i16], src_rate: u32, channels: u16) -> Vec<u8> {
    let ch = channels.max(1) as usize;
    let frames = samples.len() / ch;
    if frames == 0 {
        return wav_bytes(&[], 16000, 1);
    }
    // 1) 交错帧 → 单声道（平均下混）
    let mono: Vec<f32> = (0..frames)
        .map(|f| {
            let mut acc = 0f64;
            for c in 0..ch {
                acc += samples[f * ch + c] as f64 / 32768.0;
            }
            (acc / ch as f64) as f32
        })
        .collect();
    // 2) 线性插值重采样 src_rate → 16000（语音转写足够；whisper 内部还会再走重采样窗）
    let out: Vec<i16> = if src_rate == 16000 {
        mono.iter()
            .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect()
    } else {
        let out_frames = (frames as u64 * 16000) / src_rate as u64;
        let step = f64::from(src_rate) / 16000.0;
        (0..out_frames)
            .map(|i| {
                let pos = i as f64 * step;
                let i0 = (pos as usize).min(frames - 1);
                let i1 = (i0 + 1).min(frames - 1);
                let fr = (pos - i0 as f64) as f32;
                let v = mono[i0] * (1.0 - fr) + mono[i1] * fr;
                (v.clamp(-1.0, 1.0) * 32767.0) as i16
            })
            .collect()
    };
    wav_bytes(&out, 16000, 1)
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
