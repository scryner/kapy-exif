use anyhow::{anyhow, Result};
use async_trait::async_trait;
use core::fmt;
use log::debug;
use std::{path::Path, sync::Arc};

use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, SeekFrom},
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
    // (marker, value, has_payload)
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

pub async fn jpeg(path: impl AsRef<Path>) -> Result<impl ExtractRawExif + CopyWithRawExif> {
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
    async fn extract(&self) -> Result<Option<Vec<u8>>> {
        let mut guard = self.file.lock().await;

        // extract exif
        match extract_exif(&mut *guard, &self.structures).await? {
            Some((_, exif)) => Ok(Some(exif)),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl CopyWithRawExif for Jpeg {
    async fn copy_with_raw_exif(
        &self,
        exif: &[u8],
        mut writer: impl AsyncWrite + Send + Sync + Unpin,
    ) -> Result<()> {
        // get reader to read from original file
        let mut reader = self.file.lock().await;

        // extract exif data
        let prev_exif = extract_exif(&mut *reader, &self.structures).await?;
        let (prev_exif_marker, _) = prev_exif.ok_or_else(|| anyhow!("Exif data not found"))?;

        // write markers
        for (marker, payload) in self.structures.iter() {
            if *marker == prev_exif_marker {
                // write marker
                writer.write_u16(marker.value()).await?;

                // write length
                writer.write_u16(exif.len() as u16 + 2).await?; // additional 2 bytes means length field itself

                // write exif data
                writer.write_all(exif).await?;
            } else if *marker == Marker::SOS {
                // write SOS marker and the rest of the structures
                writer.write_u16(marker.value()).await?;

                if let Payload::NoContent(next_offset) = payload {
                    // seek to the next offset
                    reader.seek(SeekFrom::Start(*next_offset)).await?;

                    // copy reader to writer
                    tokio::io::copy(&mut *reader, &mut writer).await?;

                    return Ok(());
                } else {
                    return Err(anyhow!("Invalid payload for SOS marker"));
                }
            } else {
                // write other markers
                match payload {
                    Payload::Content(offset, length) => {
                        // write marker
                        writer.write_u16(marker.value()).await?;

                        // write length
                        writer.write_u16(*length + 2).await?; // additional 2 bytes means length field itself

                        // seek to payload
                        reader.seek(SeekFrom::Start(*offset)).await?;

                        // read payload
                        let mut payload = vec![0u8; *length as usize];
                        reader.read_exact(&mut payload).await?;

                        // write payload to writer
                        writer.write_all(&payload).await?;
                    }
                    Payload::NoContent(_) => {
                        // write marker only
                        writer.write_u16(marker.value()).await?;
                    }
                }
            }
        }

        Err(anyhow!("Invalid structure: SOS marker not found")) // actually, never reached (we've been already checked while decoding)
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
            // check first marker is SOI
            if structures[0].0 != Marker::SOI {
                // never panic: at least, SOS marker would be found
                return Err(anyhow!("Invalid SOI marker"));
            }

            return Ok(structures);
        }
    }

    Err(anyhow!("Premature end of file"))
}

async fn extract_exif<R>(
    r: &mut R,
    structures: &Vec<(Marker, Payload)>,
) -> Result<Option<(Marker, Vec<u8>)>>
where
    R: AsyncSeek + AsyncRead + Send + Sync + Unpin,
{
    // traverse structures to find APP1 marker
    for (marker, payload) in structures.iter() {
        if marker.is_app_segment() {
            match *payload {
                Payload::Content(offset, length) => {
                    // seek to the payload
                    r.seek(SeekFrom::Start(offset)).await?;

                    // make buffer to read exif
                    let mut buf = vec![0u8; length as usize];

                    // read first 4 bytes to check starting with "Exif"
                    r.read_exact(&mut buf[0..4]).await?;

                    if &buf[0..4] != b"Exif" {
                        continue;
                    }

                    // read the rest of exif
                    r.read_exact(&mut buf[4..]).await?;

                    // we got exif, return it
                    return Ok(Some((*marker, buf)));
                }
                Payload::NoContent(_) => {
                    return Err(anyhow!("Invalid payload for APP segment: {:?}", marker));
                }
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::io::SeekFrom;

    use anyhow::Result;
    use tokio::{
        fs::File,
        io::{AsyncReadExt, AsyncSeekExt, BufReader},
    };

    use crate::{
        internal::init_logger,
        jpeg::{decode_structures, jpeg},
        CopyWithRawExif, ExtractRawExif,
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
            let exif_data = jpeg
                .extract()
                .await
                .expect("Failed to extract exif")
                .expect("Exif not found");

            println!(
                "--------{} (content length: {})\n{}",
                file,
                exif_data.len(),
                hex::encode(&exif_data)
            );
        }
    }

    #[tokio::test]
    async fn copy_with_raw_exif_for_jpeg() {
        // init logger
        init_logger();

        // copy with raw exif (consequently, copy exactly the same file)
        for file in SAMPLES.into_iter() {
            // make tempfile
            let tmp = tempfile::tempfile().expect("Failed to create tempfile");
            let mut tmp = File::from_std(tmp);

            // copy with raw exif
            {
                let jpeg = jpeg(file).await.expect("Failed to open file");
                let exif_data = jpeg
                    .extract()
                    .await
                    .expect("Failed to extract exif")
                    .expect("Exif not found");
                jpeg.copy_with_raw_exif(&exif_data, &mut tmp)
                    .await
                    .expect("Failed to copy with raw exif");
            }

            // re-open the original file
            let mut orig = File::open(file).await.expect("Failed to open file");

            // compare the original file and the copied file
            assert!(compare_files(&mut orig, &mut tmp)
                .await
                .expect("Failed to compare files"));
        }
    }

    async fn compare_files(file1: &mut File, file2: &mut File) -> Result<bool> {
        const BUFFER_SIZE: usize = 8192;

        // seek to the beginning
        file1.seek(SeekFrom::Start(0)).await?;
        file2.seek(SeekFrom::Start(0)).await?;

        // make reader
        let mut reader1 = BufReader::new(file1);
        let mut reader2 = BufReader::new(file2);

        // make buffer
        let mut buffer1 = vec![0; BUFFER_SIZE];
        let mut buffer2 = vec![0; BUFFER_SIZE];

        // compare by block
        loop {
            let bytes_read1 = reader1.read(&mut buffer1).await?;
            let bytes_read2 = reader2.read(&mut buffer2).await?;

            if bytes_read1 != bytes_read2 {
                return Ok(false);
            }

            if bytes_read1 == 0 {
                break; // EOF
            }

            if buffer1[..bytes_read1] != buffer2[..bytes_read1] {
                return Ok(false);
            }
        }

        Ok(true)
    }
}
