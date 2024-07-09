use anyhow::{anyhow, Result};
use async_trait::async_trait;
use core::fmt;
use log::debug;
use std::{path::Path, sync::Arc};

use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, SeekFrom},
    sync::Mutex,
};

use crate::{CopyWithRawExif, ExtractRawExif};

macro_rules! markers {
    ($(($name:ident, $value:expr, $has_payload:expr)),*) => {
        #[allow(dead_code)]
        #[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
        enum Marker {
            $(
                $name,
            )*
        }

        impl Marker {
            fn from_value(value: u16) -> Option<Self> {
                match value {
                    $(
                        $value => Some(Marker::$name),
                    )*
                    _ => None,
                }
            }

            fn value(&self) -> u16 {
                match self {
                    $(
                        Marker::$name => $value, // big-endian, so no need to swap bytes
                    )*
                }
            }

            fn has_payload(&self) -> bool {
                match self {
                    $(
                        Marker::$name => $has_payload,
                    )*
                }
            }

            fn is_app_segment(&self) -> bool {
                match self {
                    $(
                        Marker::$name => stringify!($name).starts_with("APP"),
                    )*
                }
            }
        }
    };
}

markers!(
    (SOF0, 0xffc0, true),
    (SOF1, 0xffc1, true),
    (SOF2, 0xffc2, true),
    (SOF3, 0xffc3, true),
    (DHT, 0xffc4, true),
    (SOF5, 0xffc5, true),
    (SOF6, 0xffc6, true),
    (SOF7, 0xffc7, true),
    (SOF8, 0xffc8, true),
    (SOF9, 0xffc9, true),
    (SOF10, 0xffca, true),
    (SOF11, 0xffcb, true),
    (SOF12, 0xffcc, true),
    (SOF13, 0xffcd, true),
    (SOF14, 0xffce, true),
    (SOF15, 0xffcf, true),
    (SOI, 0xffd8, false),
    (EOI, 0xffd9, false),
    (SOS, 0xffda, false),
    (DQT, 0xffdb, true),
    (DRI, 0xffdd, true),
    (APP1, 0xffe1, true),
    (APP2, 0xffe2, true),
    (APP3, 0xffe3, true),
    (APP4, 0xffe4, true),
    (APP5, 0xffe5, true),
    (APP6, 0xffe6, true),
    (APP7, 0xffe7, true),
    (APP8, 0xffe8, true),
    (APP9, 0xffe9, true),
    (APP10, 0xffea, true),
    (APP11, 0xffeb, true),
    (APP12, 0xffec, true),
    (APP13, 0xffed, true),
    (APP14, 0xffee, true),
    (APP15, 0xffef, true),
    (COM, 0xfffe, true)
);

pub async fn jpeg(path: impl AsRef<Path>) -> Result<impl ExtractRawExif> {
    // open file
    let mut file = File::open(path.as_ref())
        .await
        .map_err(|e| anyhow!("Failed to open file: {}", e))?;

    // decode jpeg structure
    let structures = decode_structures(&mut file).await?;

    // seek to the beginning
    file.seek(SeekFrom::Start(0)).await?;

    Ok(Jpeg {
        file: Arc::new(Mutex::new(file)),
        structures,
    })
}

struct Jpeg {
    file: Arc<Mutex<File>>,
    structures: Vec<(Marker, Payload)>,
}

#[async_trait]
impl ExtractRawExif for Jpeg {
    async fn extract(&self) -> Result<Vec<u8>> {
        // traverse structures to find APP1 marker
        for (marker, payload) in self.structures.iter() {
            if marker.is_app_segment() {
                match *payload {
                    Payload::Content(offset, length) => {
                        let mut guard = self.file.lock().await;

                        // seek to the payload
                        guard.seek(SeekFrom::Start(offset)).await?;

                        // make buffer to read exif
                        let mut buf = vec![0u8; length as usize];

                        // read first 4 bytes to check starting with "Exif"
                        guard.read_exact(&mut buf[0..4]).await?;

                        if &buf[0..4] != b"Exif" {
                            continue;
                        }

                        // read the rest of exif
                        guard.read_exact(&mut buf[4..]).await?;

                        // we got exif, return it
                        return Ok(buf);
                    }
                    Payload::NoContent(_) => {
                        return Err(anyhow!("Invalid payload for APP segment: {:?}", marker));
                    }
                }
            }
        }

        Err(anyhow!("No exif segment found"))
    }
}

#[async_trait]
impl CopyWithRawExif for Jpeg {
    async fn copy_with_raw_exif(
        &self,
        exif: Vec<u8>,
        writer: impl AsyncWrite + Send + Sync,
    ) -> Result<()> {
        todo!()
    }
}

#[derive(Debug)]
enum Payload {
    Content(u64, u16),
    NoContent(u64),
}

impl fmt::Display for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Payload::Content(offset, length) => {
                write!(f, "Payload(offset: {}, length: {})", offset, length)
            }
            Payload::NoContent(offset) => {
                write!(f, "NoPayload(next_entry_offset: {})", offset)
            }
        }
    }
}

async fn decode_structures<R>(r: &mut R) -> Result<Vec<(Marker, Payload)>>
where
    R: AsyncSeek + AsyncRead + Send + Sync + Unpin,
{
    let mut structures = Vec::new();

    // seek to the beginning
    r.seek(SeekFrom::Start(0))
        .await
        .map_err(|e| anyhow!("Failed to seek: {}", e))?;

    // read first marker and check whether it's SOI
    let soi = r.read_u16().await?;
    if soi != Marker::SOI.value() {
        return Err(anyhow!("Invalid SOI marker: {:x?}", soi));
    }

    // loop through markers
    loop {
        // read marker
        let marker = match r.read_u16().await {
            Ok(v) => Marker::from_value(v).ok_or_else(|| anyhow!("Unknown marker: 0x{:x?}", v))?,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    // end of file
                    debug!("End of file: pos({})", r.stream_position().await?);
                    break;
                } else {
                    return Err(anyhow!("Failed to read marker: {}", e));
                }
            }
        };

        if !marker.has_payload() {
            let next_offset = r.stream_position().await?;

            debug!(
                "Marker: {:?}, NoContent / next offset: {}",
                marker, next_offset
            );

            structures.push((marker, Payload::NoContent(next_offset)));
        } else {
            // read length
            let payload_length = r.read_u16().await? - 2; // read length includes the length field itself
            let payload_offset = r.stream_position().await?;

            debug!(
                "Marker: {:?}, Content(offset: {}, length: {})",
                marker, payload_offset, payload_length
            );

            structures.push((marker, Payload::Content(payload_offset, payload_length)));

            // seek to the next marker
            r.seek(SeekFrom::Current(payload_length as i64))
                .await
                .map_err(|e| anyhow!("Failed to seek: {}", e))?;
        }

        // SOS marker means the end of structures of interest, so return it
        if marker == Marker::SOS {
            return Ok(structures);
        }
    }

    Err(anyhow!("Premature end of file"))
}

#[cfg(test)]
mod tests {
    use tokio::fs::File;

    use crate::{
        internal::init_logger,
        jpeg::{decode_structures, jpeg},
        ExtractRawExif,
    };

    const SAMPLES: [&str; 1] = ["sample/sample_by_pentax-k1.jpg"];

    #[tokio::test]
    async fn decode_jpeg_structures() {
        // init logger
        init_logger();

        // decode jpeg structures
        for file in SAMPLES.into_iter() {
            // open file
            let mut f = File::open(file)
                .await
                .expect(&format!("Failed to open file: {}", file));

            // decode structures
            let structures = decode_structures(&mut f)
                .await
                .expect("Failed to decode structures");

            println!("{:#?}", structures);
        }
    }

    #[tokio::test]
    async fn extract_exif_from_jpeg() {
        // init logger
        init_logger();

        // extract exif from samples
        for file in SAMPLES.into_iter() {
            let jpeg = jpeg(file).await.expect("Failed to open file");
            let exif_data = jpeg.extract().await.expect("Failed to extract exif");

            println!(
                "--------{} (content length: {})\n{}",
                file,
                exif_data.len(),
                hex::encode(&exif_data)
            );
        }
    }
}
