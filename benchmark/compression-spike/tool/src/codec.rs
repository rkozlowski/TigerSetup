//! The three codecs behind one streaming interface, single-threaded and with
//! their effective parameters recorded, so that a result names exactly what
//! produced it.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    /// Copy through: the I/O floor every other number sits on.
    Store,
    /// Raw DEFLATE from `flate2` with the zlib-rs backend — the product's own
    /// encoder and level.
    Deflate,
    /// A zstd frame (libzstd, single-threaded, no checksum, pledged size).
    Zstd,
    /// A raw LZMA2 stream (liblzma, single-threaded, no container).
    Lzma2,
}

/// One codec setting under test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Setting {
    pub codec: Codec,
    /// DEFLATE 1–9, zstd 1–22, LZMA2 preset 0–9.
    #[serde(default)]
    pub level: i32,
    /// zstd: override the window log (and enable long-distance matching, as
    /// the CLI's `--long=N` does). LZMA2: override the dictionary size as a
    /// log2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_log: Option<u32>,
    /// LZMA2: the preset's extreme variant (`9e`).
    #[serde(default)]
    pub extreme: bool,
    /// LZMA2: an x86 BCJ branch-converter filter ahead of LZMA2, the chain
    /// 7-Zip uses for executables.
    #[serde(default)]
    pub bcj: bool,
}

impl Setting {
    pub fn label(&self) -> String {
        let base = match self.codec {
            Codec::Store => "store".to_string(),
            Codec::Deflate => format!("deflate-{}", self.level),
            Codec::Zstd => format!("zstd-{}", self.level),
            Codec::Lzma2 => format!(
                "lzma2-{}{}",
                self.level,
                if self.extreme { "e" } else { "" }
            ),
        };
        let base = match self.window_log {
            Some(w) => format!("{base}-w{w}"),
            None => base,
        };
        if self.bcj {
            format!("{base}-bcj")
        } else {
            base
        }
    }
}

/// The parameters the codec actually ran with, as it reports them.
pub fn effective_parameters(setting: &Setting, pledged: u64) -> serde_json::Value {
    match setting.codec {
        Codec::Store => serde_json::json!({}),
        Codec::Deflate => serde_json::json!({
            "backend": "flate2/zlib-rs",
            "level": setting.level,
            "window": 32768,
        }),
        Codec::Zstd => {
            // ZSTD_getCParams is the table the library itself consults for a
            // level and a source size.
            let params = unsafe { zstd_sys::ZSTD_getCParams(setting.level, pledged, 0) };
            let window_log = setting.window_log.unwrap_or(params.windowLog);
            serde_json::json!({
                "library": format!("libzstd {}", zstd::zstd_safe::version_string()),
                "level": setting.level,
                "windowLog": window_log,
                "windowBytes": 1u64 << window_log,
                "chainLog": params.chainLog,
                "hashLog": params.hashLog,
                "searchLog": params.searchLog,
                "minMatch": params.minMatch,
                "targetLength": params.targetLength,
                "strategy": zstd_strategy_name(params.strategy as u32),
                "longDistanceMatching": setting.window_log.is_some(),
                "nbWorkers": 0,
                "checksum": false,
                "pledgedSrcSize": pledged,
            })
        }
        Codec::Lzma2 => {
            let mut raw = unsafe { std::mem::zeroed::<liblzma_sys::lzma_options_lzma>() };
            let preset = lzma_preset(setting);
            let ok = unsafe { liblzma_sys::lzma_lzma_preset(&mut raw, preset) };
            let dict_size = setting
                .window_log
                .map(|w| 1u32 << w)
                .unwrap_or(raw.dict_size);
            serde_json::json!({
                "library": format!("liblzma {}", lzma_version()),
                "preset": setting.level,
                "extreme": setting.extreme,
                "presetAccepted": ok == 0,
                "dictSize": dict_size,
                "lc": raw.lc,
                "lp": raw.lp,
                "pb": raw.pb,
                "mode": if raw.mode == liblzma_sys::lzma_mode_LZMA_MODE_NORMAL { "normal" } else { "fast" },
                "niceLen": raw.nice_len,
                "matchFinder": lzma_mf_name(raw.mf),
                "depth": raw.depth,
                "container": "raw LZMA2 (no .xz container, no check)",
                "filters": if setting.bcj { "x86 BCJ + LZMA2" } else { "LZMA2" },
                "threads": 1,
            })
        }
    }
}

fn zstd_strategy_name(strategy: u32) -> &'static str {
    match strategy {
        1 => "fast",
        2 => "dfast",
        3 => "greedy",
        4 => "lazy",
        5 => "lazy2",
        6 => "btlazy2",
        7 => "btopt",
        8 => "btultra",
        9 => "btultra2",
        _ => "unknown",
    }
}

fn lzma_mf_name(mf: liblzma_sys::lzma_match_finder) -> &'static str {
    match mf {
        liblzma_sys::lzma_match_finder_LZMA_MF_HC3 => "hc3",
        liblzma_sys::lzma_match_finder_LZMA_MF_HC4 => "hc4",
        liblzma_sys::lzma_match_finder_LZMA_MF_BT2 => "bt2",
        liblzma_sys::lzma_match_finder_LZMA_MF_BT3 => "bt3",
        liblzma_sys::lzma_match_finder_LZMA_MF_BT4 => "bt4",
        _ => "unknown",
    }
}

fn lzma_version() -> String {
    let raw = unsafe { std::ffi::CStr::from_ptr(liblzma_sys::lzma_version_string()) };
    raw.to_string_lossy().into_owned()
}

fn lzma_preset(setting: &Setting) -> u32 {
    let mut preset = setting.level as u32;
    if setting.extreme {
        preset |= liblzma_sys::LZMA_PRESET_EXTREME;
    }
    preset
}

fn lzma_filters(setting: &Setting) -> liblzma::stream::Filters {
    let mut filters = liblzma::stream::Filters::new();
    if setting.bcj {
        filters.x86();
    }
    filters.lzma2(&lzma_options(setting));
    filters
}

fn lzma_options(setting: &Setting) -> liblzma::stream::LzmaOptions {
    let mut options = liblzma::stream::LzmaOptions::new_preset(lzma_preset(setting))
        .expect("a valid LZMA preset");
    if let Some(w) = setting.window_log {
        options.dict_size(1u32 << w);
    }
    options
}

/// A streaming encoder writing to `sink`.
pub enum Encoder<W: Write> {
    Store(W),
    Deflate(flate2::write::DeflateEncoder<W>),
    Zstd(zstd::stream::write::Encoder<'static, W>),
    Lzma2(liblzma::write::XzEncoder<W>),
}

impl<W: Write> Encoder<W> {
    pub fn new(setting: &Setting, sink: W, pledged: u64) -> io::Result<Self> {
        Ok(match setting.codec {
            Codec::Store => Encoder::Store(sink),
            Codec::Deflate => Encoder::Deflate(flate2::write::DeflateEncoder::new(
                sink,
                flate2::Compression::new(setting.level as u32),
            )),
            Codec::Zstd => {
                let mut encoder = zstd::stream::write::Encoder::new(sink, setting.level)?;
                if let Some(w) = setting.window_log {
                    encoder.set_parameter(zstd::stream::raw::CParameter::WindowLog(w))?;
                    encoder.long_distance_matching(true)?;
                }
                encoder.set_pledged_src_size(Some(pledged))?;
                encoder.include_checksum(false)?;
                encoder.include_contentsize(true)?;
                Encoder::Zstd(encoder)
            }
            Codec::Lzma2 => {
                let stream = liblzma::stream::Stream::new_raw_encoder(&lzma_filters(setting))
                    .map_err(|e| io::Error::other(format!("lzma raw encoder: {e}")))?;
                Encoder::Lzma2(liblzma::write::XzEncoder::new_stream(sink, stream))
            }
        })
    }

    pub fn finish(self) -> io::Result<W> {
        match self {
            Encoder::Store(w) => Ok(w),
            Encoder::Deflate(e) => e.finish(),
            Encoder::Zstd(e) => e.finish(),
            Encoder::Lzma2(e) => e.finish(),
        }
    }
}

impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Encoder::Store(w) => w.write(buf),
            Encoder::Deflate(e) => e.write(buf),
            Encoder::Zstd(e) => e.write(buf),
            Encoder::Lzma2(e) => e.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Encoder::Store(w) => w.flush(),
            Encoder::Deflate(e) => e.flush(),
            Encoder::Zstd(e) => e.flush(),
            Encoder::Lzma2(e) => e.flush(),
        }
    }
}

/// A streaming decoder reading from `source`.
pub enum Decoder<R: Read> {
    Store(R),
    Deflate(flate2::read::DeflateDecoder<R>),
    Zstd(zstd::stream::read::Decoder<'static, io::BufReader<R>>),
    Lzma2(liblzma::read::XzDecoder<R>),
}

impl<R: Read> Decoder<R> {
    pub fn new(setting: &Setting, source: R) -> io::Result<Self> {
        Ok(match setting.codec {
            Codec::Store => Decoder::Store(source),
            Codec::Deflate => Decoder::Deflate(flate2::read::DeflateDecoder::new(source)),
            Codec::Zstd => {
                let mut decoder = zstd::stream::read::Decoder::new(source)?;
                if let Some(w) = setting.window_log {
                    decoder.window_log_max(w)?;
                }
                Decoder::Zstd(decoder)
            }
            Codec::Lzma2 => {
                let stream = liblzma::stream::Stream::new_raw_decoder(&lzma_filters(setting))
                    .map_err(|e| io::Error::other(format!("lzma raw decoder: {e}")))?;
                Decoder::Lzma2(liblzma::read::XzDecoder::new_stream(source, stream))
            }
        })
    }
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Decoder::Store(r) => r.read(buf),
            Decoder::Deflate(d) => d.read(buf),
            Decoder::Zstd(d) => d.read(buf),
            Decoder::Lzma2(d) => d.read(buf),
        }
    }
}
