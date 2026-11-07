//! Bounded asynchronous stream reading and buffer management.

use crate::StreamCallback;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024; // 10MB limit

/// Finds the highest byte index `<= index` that lies on a valid UTF-8 character boundary.
pub fn floor_char_boundary(slice: &[u8], mut index: usize) -> usize {
    if index >= slice.len() {
        return slice.len();
    }
    // In UTF-8, continuation bytes have the pattern 10xxxxxx (0x80..=0xBF).
    while index > 0 && (slice[index] & 0b1100_0000) == 0b1000_0000 {
        index -= 1;
    }
    index
}

/// Finds the lowest byte index `>= index` that lies on a valid UTF-8 character boundary.
pub fn ceil_char_boundary(slice: &[u8], mut index: usize) -> usize {
    if index >= slice.len() {
        return slice.len();
    }
    while index < slice.len() && (slice[index] & 0b1100_0000) == 0b1000_0000 {
        index += 1;
    }
    index
}

/// Safely truncates a UTF-8 string slice to at most `max_bytes` without splitting codepoints.
pub fn safe_truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let boundary = floor_char_boundary(s.as_bytes(), max_bytes);
    &s[..boundary]
}

/// Safely truncates a byte slice at a valid UTF-8 character boundary.
pub fn safe_truncate_bytes(slice: &[u8], max_bytes: usize) -> &[u8] {
    if slice.len() <= max_bytes {
        return slice;
    }
    let boundary = floor_char_boundary(slice, max_bytes);
    &slice[..boundary]
}

pub async fn read_stream(
    stream: impl AsyncRead + Unpin,
    callback: Option<StreamCallback>,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    let effective_max_bytes = if max_bytes == 0 {
        DEFAULT_MAX_OUTPUT_BYTES
    } else {
        max_bytes
    };

    let mut reader = BufReader::new(stream);
    let mut output = Vec::new();
    let mut chunk = Vec::new();
    let mut truncated = false;
    loop {
        chunk.clear();
        // Bound the temporary line buffer too: read_until alone can allocate without
        // limit when a subprocess emits a very long line without a newline.
        let line_limit = effective_max_bytes.saturating_add(1).min(u64::MAX as usize) as u64;
        if (&mut reader)
            .take(line_limit)
            .read_until(b'\n', &mut chunk)
            .await?
            == 0
        {
            break;
        }
        if chunk.len() > effective_max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "process output line exceeds configured limit",
            ));
        }
        if let Some(cb) = &callback {
            let chunk_ref = &chunk;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cb(&String::from_utf8_lossy(chunk_ref));
            }));
        }
        if output.len() + chunk.len() <= effective_max_bytes {
            output.extend_from_slice(&chunk);
        } else if !truncated {
            let remaining = effective_max_bytes.saturating_sub(output.len());
            if remaining > 0 {
                let safe_len = floor_char_boundary(&chunk, remaining);
                if safe_len > 0 {
                    output.extend_from_slice(&chunk[..safe_len]);
                }
            }
            output.extend_from_slice(b"\n...[OUTPUT TRUNCATED: buffer exceeded limit]...\n");
            truncated = true;
        }
    }
    Ok(output)
}

pub async fn read_stream_string(
    stream: impl AsyncRead + Unpin,
    callback: Option<StreamCallback>,
    max_bytes: usize,
) -> std::io::Result<String> {
    let bytes = read_stream(stream, callback, max_bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
