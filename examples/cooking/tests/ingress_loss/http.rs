use super::MAX_HTTP_MESSAGE_BYTES;
use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) fn caddy_host(url: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let parsed = reqwest::Url::parse(url)?;
    let host = parsed.host_str().ok_or("Caddy URL has no host")?;
    Ok(parsed
        .port()
        .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}")))
}

pub(crate) async fn read_http_message<S>(
    stream: &mut S,
    timeout: Duration,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + Unpin,
{
    let mut data = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;
    loop {
        let read = tokio::time::timeout(timeout, stream.read(&mut chunk)).await??;
        if read == 0 {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "HTTP headers truncated").into(),
            );
        }
        data.extend_from_slice(&chunk[..read]);
        if data.len() > MAX_HTTP_MESSAGE_BYTES {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "HTTP message exceeds bound").into(),
            );
        }
        if let Some(position) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
    }
    let headers = &data[..header_end];
    let content_length = headers
        .split(|byte| *byte == b'\n')
        .find_map(|line| {
            let line = std::str::from_utf8(line).ok()?.trim();
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or("HTTP message lacks bounded Content-Length")?;
    let message_end = header_end
        .checked_add(content_length)
        .filter(|end| *end <= MAX_HTTP_MESSAGE_BYTES)
        .ok_or("HTTP message exceeds bound")?;
    while data.len() < message_end {
        let remaining = message_end - data.len();
        let read_len = remaining.min(chunk.len());
        let read = tokio::time::timeout(timeout, stream.read(&mut chunk[..read_len])).await??;
        if read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "HTTP body truncated").into());
        }
        data.extend_from_slice(&chunk[..read]);
    }
    data.truncate(message_end);
    Ok(data)
}
