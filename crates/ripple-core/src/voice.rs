//! Voice-note recording: microphone → 48 kHz mono → Opus → Ogg.
//!
//! The audio stream lives on its own thread because platform audio handles
//! are not `Send`. Samples accumulate in memory (a few MB per minute) and are
//! encoded when the recording is finished.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const RATE: u32 = 48_000;
const FRAME: usize = 960; // 20 ms
/// Hard stop so a forgotten recording cannot grow without bound (15 minutes).
const MAX_SAMPLES: usize = RATE as usize * 60 * 15;

pub struct Take {
    pub ogg: Vec<u8>,
    pub duration_ms: u64,
    /// 64 amplitude samples, 0–100.
    pub waveform: Vec<u8>,
}

struct Shared {
    samples: Mutex<Vec<f32>>,
    stop: AtomicBool,
}

pub struct Recorder {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    in_rate: u32,
}

impl Recorder {
    /// Starts capturing. `on_level(level 0–100, elapsed ms)` fires ~16×/s.
    pub fn start(on_level: impl Fn(u8, u32) + Send + 'static) -> Result<Recorder> {
        let shared = Arc::new(Shared { samples: Mutex::new(Vec::new()), stop: AtomicBool::new(false) });
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32>>();
        let sh = shared.clone();
        let thread = std::thread::Builder::new().name("ripple-mic".into()).spawn(move || {
            let stream = match open_stream(sh.clone()) {
                Ok((stream, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    stream
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let began = Instant::now();
            let mut seen = 0usize;
            while !sh.stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(60));
                let (level, len) = {
                    let s = sh.samples.lock().unwrap_or_else(|e| e.into_inner());
                    let recent = &s[seen.min(s.len())..];
                    let rms = if recent.is_empty() {
                        0.0
                    } else {
                        (recent.iter().map(|x| x * x).sum::<f32>() / recent.len() as f32).sqrt()
                    };
                    ((rms * 320.0).min(100.0) as u8, s.len())
                };
                seen = len;
                on_level(level, began.elapsed().as_millis() as u32);
                if len >= MAX_SAMPLES {
                    break;
                }
            }
            drop(stream);
        })?;
        let in_rate = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| anyhow!("The microphone did not respond"))??;
        Ok(Recorder { shared, thread: Some(thread), in_rate })
    }

    /// Stops capturing and encodes the take.
    pub fn finish(mut self) -> Result<Take> {
        self.halt();
        let raw = std::mem::take(&mut *self.shared.samples.lock().unwrap_or_else(|e| e.into_inner()));
        let pcm = resample(&raw, self.in_rate, RATE);
        if pcm.is_empty() {
            bail!("Nothing was recorded. Check microphone access in your system settings.");
        }
        let duration_ms = pcm.len() as u64 * 1000 / RATE as u64;
        Ok(Take { ogg: encode_ogg_opus(&pcm)?, duration_ms, waveform: waveform(&pcm, 64) })
    }

    fn halt(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.halt();
    }
}

fn open_stream(shared: Arc<Shared>) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow!("No microphone found"))?;
    let supported = device.default_input_config().context("Could not read the microphone format")?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let err = |e| log::warn!("microphone stream error: {e}");

    fn push<T: Copy>(shared: &Shared, data: &[T], channels: usize, conv: impl Fn(T) -> f32) {
        let mut s = shared.samples.lock().unwrap_or_else(|e| e.into_inner());
        if s.len() >= MAX_SAMPLES {
            return;
        }
        for frame in data.chunks(channels.max(1)) {
            let sum: f32 = frame.iter().map(|x| conv(*x)).sum();
            s.push(sum / frame.len() as f32);
        }
    }

    let stream = match format {
        cpal::SampleFormat::F32 => {
            let sh = shared.clone();
            device.build_input_stream(config, move |d: &[f32], _: &_| push(&sh, d, channels, |x| x), err, None)
        }
        cpal::SampleFormat::I16 => {
            let sh = shared.clone();
            device.build_input_stream(
                config,
                move |d: &[i16], _: &_| push(&sh, d, channels, |x| x as f32 / 32768.0),
                err,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let sh = shared.clone();
            device.build_input_stream(
                config,
                move |d: &[u16], _: &_| push(&sh, d, channels, |x| (x as f32 - 32768.0) / 32768.0),
                err,
                None,
            )
        }
        cpal::SampleFormat::I32 => {
            let sh = shared.clone();
            device.build_input_stream(
                config,
                move |d: &[i32], _: &_| push(&sh, d, channels, |x| x as f32 / 2_147_483_648.0),
                err,
                None,
            )
        }
        other => bail!("Unsupported microphone sample format: {other:?}"),
    }
    .context("Could not open the microphone")?;
    stream.play().context("Could not start the microphone")?;
    Ok((stream, rate))
}

/// Linear-interpolation resampler; plenty for speech.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let i0 = pos as usize;
            let i1 = (i0 + 1).min(input.len() - 1);
            let t = (pos - i0 as f64) as f32;
            input[i0] * (1.0 - t) + input[i1] * t
        })
        .collect()
}

/// Peak amplitude per bucket, normalised to 0–100.
pub fn waveform(pcm: &[f32], buckets: usize) -> Vec<u8> {
    if pcm.is_empty() || buckets == 0 {
        return vec![];
    }
    let size = pcm.len().div_ceil(buckets).max(1);
    let peaks: Vec<f32> = pcm.chunks(size).map(|c| c.iter().fold(0f32, |a, x| a.max(x.abs()))).collect();
    let top = peaks.iter().cloned().fold(0.0001f32, f32::max);
    let mut out: Vec<u8> = peaks.iter().map(|p| ((p / top) * 100.0).round() as u8).collect();
    out.resize(buckets, 0);
    out
}

/// Encodes 48 kHz mono float PCM as an Ogg Opus file.
pub fn encode_ogg_opus(pcm: &[f32]) -> Result<Vec<u8>> {
    use ogg::writing::{PacketWriteEndInfo, PacketWriter};

    let mut enc = ropus::Encoder::builder(RATE, ropus::Channels::Mono, ropus::Application::Voip)
        .bitrate(ropus::Bitrate::try_bits(32_000).map_err(|e| anyhow!("opus bitrate: {e:?}"))?)
        .build()
        .map_err(|e| anyhow!("opus encoder: {e:?}"))?;
    let pre_skip = enc.lookahead() as u16;
    let serial = rand::random::<u32>();

    let mut out = Vec::with_capacity(pcm.len() / 12);
    {
        let mut w = PacketWriter::new(&mut out);
        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family
        w.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;

        let vendor = b"ripple";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        w.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;

        let total_frames = pcm.len().div_ceil(FRAME).max(1);
        let mut buf = [0u8; 1500];
        let mut frame = [0f32; FRAME];
        for i in 0..total_frames {
            let start = i * FRAME;
            let chunk = &pcm[start.min(pcm.len())..(start + FRAME).min(pcm.len())];
            frame[..chunk.len()].copy_from_slice(chunk);
            frame[chunk.len()..].fill(0.0);
            let n = enc.encode_float(&frame, &mut buf).map_err(|e| anyhow!("opus encode: {e:?}"))?;
            let last = i + 1 == total_frames;
            // Granule position counts decoded samples, including the pre-skip.
            let granule = if last { pcm.len() as u64 + pre_skip as u64 } else { ((i + 1) * FRAME) as u64 };
            let end = if last { PacketWriteEndInfo::EndStream } else { PacketWriteEndInfo::NormalPacket };
            w.write_packet(buf[..n].to_vec(), serial, end, granule)?;
        }
    }
    Ok(out)
}

/// Decodes an Ogg Opus file to a 16-bit mono WAV, for webviews that cannot
/// play Ogg Opus natively.
pub fn ogg_opus_to_wav(ogg: &[u8]) -> Result<Vec<u8>> {
    let mut rd = ogg::reading::PacketReader::new(std::io::Cursor::new(ogg));
    let mut dec = ropus::Decoder::new(RATE, ropus::Channels::Mono).map_err(|e| anyhow!("opus decoder: {e:?}"))?;
    let mut pcm: Vec<i16> = Vec::new();
    let mut buf = vec![0i16; 5760];
    let mut pre_skip = 0usize;
    let mut index = 0;
    while let Some(p) = rd.read_packet().context("corrupt ogg stream")? {
        index += 1;
        if index == 1 {
            if p.data.len() < 12 || &p.data[..8] != b"OpusHead" {
                bail!("not an Ogg Opus file");
            }
            pre_skip = u16::from_le_bytes([p.data[10], p.data[11]]) as usize;
            continue;
        }
        if index == 2 {
            continue; // OpusTags
        }
        if let Ok(n) = dec.decode(&p.data, &mut buf, ropus::DecodeMode::Normal) {
            pcm.extend_from_slice(&buf[..n]);
        }
    }
    let pcm = &pcm[pre_skip.min(pcm.len())..];
    let data_len = (pcm.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32) -> Vec<f32> {
        (0..(RATE as f32 * secs) as usize)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin() * 0.4 * (i as f32 / 4000.0).min(1.0))
            .collect()
    }

    #[test]
    fn encodes_a_valid_ogg_opus_stream() {
        let pcm = tone(1.25);
        let ogg = encode_ogg_opus(&pcm).unwrap();
        assert_eq!(&ogg[0..4], b"OggS");
        assert!(ogg.windows(8).any(|w| w == b"OpusHead"));
        assert!(ogg.windows(8).any(|w| w == b"OpusTags"));
        // ~32 kbps: 1.25 s should land well under 12 kB and above 1 kB.
        assert!(ogg.len() > 1_000 && ogg.len() < 12_000, "unexpected size {}", ogg.len());

        // Read it back: packet count matches the 20 ms framing.
        let mut rd = ogg::reading::PacketReader::new(std::io::Cursor::new(&ogg));
        let mut packets = 0;
        let mut last_granule = 0;
        while let Some(p) = rd.read_packet().unwrap() {
            packets += 1;
            last_granule = p.absgp_page();
        }
        assert_eq!(packets, 2 + pcm.len().div_ceil(FRAME));
        assert!(last_granule >= pcm.len() as u64);

        // And decode it: same length (within a frame), and it is not silence.
        let wav = ogg_opus_to_wav(&ogg).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        let decoded = (wav.len() - 44) / 2;
        assert!((decoded as i64 - pcm.len() as i64).abs() <= FRAME as i64, "decoded {decoded} of {}", pcm.len());
        let peak = wav[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 3000, "decoded audio is too quiet: {peak}");
        assert!(ogg_opus_to_wav(b"not ogg").is_err());
    }

    #[test]
    fn resample_and_waveform() {
        let a = tone(0.5);
        let down = resample(&a, 48_000, 16_000);
        assert!((down.len() as i64 - (a.len() / 3) as i64).abs() <= 1);
        assert_eq!(resample(&a, 48_000, 48_000).len(), a.len());
        let w = waveform(&a, 64);
        assert_eq!(w.len(), 64);
        assert!(w.iter().any(|v| *v == 100));
        assert!(w[0] < w[63]);
        assert!(waveform(&[], 64).is_empty());
    }
}
