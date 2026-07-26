use serde::{Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::protocol::MAX_FRAME_SIZE;

pub fn write_sync<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let payload = encode(message)?;
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame exceeds u32 length"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

pub fn read_sync<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_be_bytes(length))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"))?;
    let payload = read_payload_sync(reader, length)?;
    decode(&payload)
}

pub async fn write_async<T: Serialize + Sync>(
    writer: &mut (impl AsyncWrite + Unpin + Send),
    message: &T,
) -> io::Result<()> {
    let payload = encode(message)?;
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame exceeds u32 length"))?;
    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

pub async fn read_async<T: DeserializeOwned + Send>(
    reader: &mut (impl AsyncRead + Unpin + Send),
) -> io::Result<T> {
    let length = reader.read_u32().await?;
    let length = usize::try_from(length)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"))?;
    let payload = read_payload_async(reader, length).await?;
    decode(&payload)
}

fn encode<T: Serialize>(message: &T) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    ciborium::into_writer(message, &mut payload).map_err(io::Error::other)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds protocol limit",
        ));
    }
    Ok(payload)
}

fn decode<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    ciborium::from_reader(payload).map_err(io::Error::other)
}

fn read_payload_sync(reader: &mut impl Read, length: usize) -> io::Result<Vec<u8>> {
    validate_length(length)?;
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

async fn read_payload_async(
    reader: &mut (impl AsyncRead + Unpin),
    length: usize,
) -> io::Result<Vec<u8>> {
    validate_length(length)?;
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

fn validate_length(length: usize) -> io::Result<()> {
    if length > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds protocol limit",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{read_sync, write_sync};
    use crate::protocol::{GuestMessage, PROTOCOL_VERSION};
    use std::io::Cursor;

    #[test]
    fn sync_frame_round_trip() {
        let expected = GuestMessage::Hello {
            version: PROTOCOL_VERSION,
        };
        let mut bytes = Vec::new();
        write_sync(&mut bytes, &expected).unwrap();

        let decoded: GuestMessage = read_sync(&mut Cursor::new(bytes)).unwrap();
        assert!(matches!(
            decoded,
            GuestMessage::Hello {
                version: PROTOCOL_VERSION
            }
        ));
    }
}
