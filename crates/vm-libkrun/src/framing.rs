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
    use crate::protocol::{
        GuestCommandMessage, GuestLogStream, GuestMessage, GuestMount, HostMessage, MAX_FRAME_SIZE,
        PROTOCOL_VERSION,
    };
    use serde::Serialize;
    use std::{collections::BTreeMap, io::Cursor, path::PathBuf};

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

    #[test]
    fn every_host_message_round_trips() {
        let command = GuestCommandMessage {
            program: String::from("/bin/echo"),
            args: vec![String::from("hello")],
            env: BTreeMap::from([(String::from("LANG"), String::from("C"))]),
            working_dir: Some(PathBuf::from("/workspace")),
        };
        let messages = [
            HostMessage::Start {
                version: PROTOCOL_VERSION,
                command,
                mounts: vec![GuestMount {
                    tag: String::from("workspace"),
                    guest_path: PathBuf::from("/workspace"),
                    read_only: false,
                }],
            },
            HostMessage::Cancel { timeout_ms: 500 },
            HostMessage::HealthPing { nonce: 42 },
        ];
        for message in messages {
            round_trip(&message);
        }
    }

    #[test]
    fn every_guest_message_round_trips() {
        let messages = [
            GuestMessage::Hello {
                version: PROTOCOL_VERSION,
            },
            GuestMessage::Ready,
            GuestMessage::Log {
                stream: GuestLogStream::Stderr,
                bytes: vec![0, 0xff, b'\n'],
            },
            GuestMessage::Metric {
                name: String::from("cpu.seconds"),
                value: 1.5,
                labels: BTreeMap::from([(String::from("scope"), String::from("guest"))]),
            },
            GuestMessage::Health { nonce: 42 },
            GuestMessage::Exited {
                code: Some(0),
                signal: None,
            },
            GuestMessage::Error {
                code: String::from("guest-test"),
                message: String::from("deliberate error"),
            },
        ];
        for message in messages {
            round_trip(&message);
        }
    }

    #[test]
    fn malformed_truncated_and_oversized_frames_are_rejected() {
        assert!(read_sync::<GuestMessage>(&mut Cursor::new([0_u8; 2])).is_err());
        let truncated = [0, 0, 0, 4, 0xa1];
        assert!(read_sync::<GuestMessage>(&mut Cursor::new(truncated)).is_err());
        let oversized = u32::try_from(MAX_FRAME_SIZE + 1).unwrap().to_be_bytes();
        assert!(read_sync::<GuestMessage>(&mut Cursor::new(oversized)).is_err());
        let malformed = [0, 0, 0, 1, 0xff];
        assert!(read_sync::<GuestMessage>(&mut Cursor::new(malformed)).is_err());
    }

    #[test]
    fn unknown_wire_variant_is_rejected_explicitly() {
        #[derive(Serialize)]
        enum FutureMessage {
            FutureVariant,
        }

        let mut bytes = Vec::new();
        write_sync(&mut bytes, &FutureMessage::FutureVariant).unwrap();
        assert!(read_sync::<GuestMessage>(&mut Cursor::new(bytes)).is_err());
    }

    fn round_trip<T>(message: &T)
    where
        T: Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let mut bytes = Vec::new();
        write_sync(&mut bytes, message).unwrap();
        let decoded: T = read_sync(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(format!("{decoded:?}"), format!("{message:?}"));
    }
}
