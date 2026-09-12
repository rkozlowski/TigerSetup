//! The container the WinGet source writes its per-package version data in:
//! the buffer format of the Windows Compression API with the MSZIP
//! algorithm (`Cabinet.dll`, `COMPRESS_ALGORITHM_MSZIP`), as observed on the
//! served files and on buffers produced by the API itself.
//!
//! ```text
//! offset  0  u32  magic 0xC0E5510A
//! offset  4  u16  header length (24)
//! offset  6  u16  unknown (varies per file; not validated)
//! offset  8  u64  uncompressed length
//! offset 16  u64  block length (32768: MSZIP's uncompressed block size)
//! offset 24  blocks: u32 compressed length, then "CK" and a raw DEFLATE
//!            stream for up to one block of output
//! ```
//!
//! Each block is its own DEFLATE stream, but the LZ77 history carries over:
//! a block may reference bytes of the previous block's output, so every
//! block after the first is inflated with the previous 32 KiB of output as
//! its dictionary.

use flate2::{Decompress, FlushDecompress, Status};

const MAGIC: u32 = 0xC0E5_510A;
const HEADER_LEN: usize = 24;
const BLOCK_SIGNATURE: &[u8; 2] = b"CK";
const WINDOW: usize = 32 * 1024;

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(data: &[u8], at: usize) -> Option<u64> {
    data.get(at..at + 8)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// Inflates a whole MSZIP buffer. Refuses more than `max_len` bytes of
/// output.
pub fn decompress(data: &[u8], max_len: usize) -> Result<Vec<u8>, String> {
    if u32_at(data, 0) != Some(MAGIC) {
        return Err("not an MSZIP buffer (bad magic)".into());
    }
    let header_len = data
        .get(4..6)
        .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
        .filter(|len| *len >= HEADER_LEN)
        .ok_or("not an MSZIP buffer (bad header length)")?;
    let uncompressed = u64_at(data, 8).ok_or("truncated MSZIP header")? as usize;
    let block_len = u64_at(data, 16).ok_or("truncated MSZIP header")? as usize;
    if uncompressed > max_len {
        return Err(format!(
            "MSZIP output of {uncompressed} bytes exceeds {max_len}"
        ));
    }
    if block_len == 0 || block_len > 1 << 20 {
        return Err(format!("MSZIP block length {block_len} is not plausible"));
    }
    let mut out: Vec<u8> = Vec::with_capacity(uncompressed);
    let mut inflater = Decompress::new(false);
    let mut at = header_len;
    let mut first = true;
    while at < data.len() {
        let compressed_len = u32_at(data, at).ok_or("truncated MSZIP block length")? as usize;
        at += 4;
        let block = data
            .get(at..at + compressed_len)
            .ok_or("truncated MSZIP block")?;
        at += compressed_len;
        if block.len() < 2 || &block[..2] != BLOCK_SIGNATURE {
            return Err("MSZIP block without its CK signature".into());
        }
        if !first {
            inflater.reset(false);
            let start = out.len().saturating_sub(WINDOW);
            inflater
                .set_dictionary(&out[start..])
                .map_err(|err| format!("cannot chain MSZIP blocks: {err}"))?;
        }
        first = false;
        let mut chunk: Vec<u8> = Vec::with_capacity(block_len + 1);
        let status = inflater
            .decompress_vec(&block[2..], &mut chunk, FlushDecompress::Finish)
            .map_err(|err| format!("MSZIP block does not inflate: {err}"))?;
        if status != Status::StreamEnd {
            return Err("MSZIP block is larger than its declared block length".into());
        }
        out.extend_from_slice(&chunk);
        if out.len() > uncompressed {
            return Err("MSZIP output exceeds its declared length".into());
        }
    }
    if out.len() != uncompressed {
        return Err(format!(
            "MSZIP output is {} bytes, the header declares {uncompressed}",
            out.len()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEBVIEW2: &[u8] =
        include_bytes!("../tests/fixtures/Microsoft.EdgeWebView2Runtime.versionData.mszyml");
    const MULTIBLOCK: &[u8] = include_bytes!("../tests/fixtures/multiblock.mszyml");

    #[test]
    fn served_single_block_version_data_inflates() {
        let out = decompress(WEBVIEW2, 1 << 20).unwrap();
        assert_eq!(out.len(), 22_614);
        let text = std::str::from_utf8(&out).unwrap();
        assert!(text.starts_with("sV: 1.0\nvD:\n- v: 152.0.4191.53\n"));
        assert_eq!(text.matches("\n- v: ").count(), 141);
    }

    #[test]
    fn blocks_chain_their_history() {
        // Two blocks produced by the Compression API itself from a 41,712
        // byte document; the second block back-references the first.
        let out = decompress(MULTIBLOCK, 1 << 20).unwrap();
        assert_eq!(out.len(), 41_712);
        assert_eq!(
            tigersetup_format::hex(&tigersetup_format::sha256(&out)),
            "39574915f8c842add8564ce4b1aedefb1ee3c98d9d8aebb98aa9c84375eed8a7"
        );
        let text = std::str::from_utf8(&out).unwrap();
        assert!(text.starts_with("sV: 1.0\nvD:\n- v: 1.0.0\n"));
        assert_eq!(text.matches("\n- v: ").count(), 320);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn damaged_buffers_are_refused() {
        assert!(decompress(b"nope", 1 << 20).is_err());
        assert!(decompress(&WEBVIEW2[..100], 1 << 20).is_err());
        assert!(decompress(WEBVIEW2, 1000).is_err());
        let mut broken = WEBVIEW2.to_vec();
        broken[28] = b'X';
        assert!(decompress(&broken, 1 << 20).is_err());
    }
}
