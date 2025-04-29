use std::io::SeekFrom;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::debug;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::heic::Heic;
use crate::ExtractRawExif;

#[async_trait]
impl ExtractRawExif for Heic {
    async fn extract(&self) -> Result<Option<Vec<u8>>> {
        // get offset and length of exif box
        let (_, (exif_ptr, _)) = self.exif_ptr().ok_or(anyhow!("Exif item not found"))?;
        debug!("exif ptr: {:?}", exif_ptr);

        // check exif length to avoid OOM (maybe, due to malicious file or uncaught errors on parsing ISOBMFF format)
        const MAX_LEN: usize = 1024 * 1024 * 8; // 8MB

        if (exif_ptr.length - 4) > MAX_LEN {
            // I don't know why -4 is needed currently (it may be another header)
            return Err(anyhow!("Exif length is too large: {}", exif_ptr.length - 4));
        }

        // read exif content from file
        let mut guard = self.file.lock().await;

        // seek to raw exif content
        guard.seek(SeekFrom::Start(exif_ptr.offset + 4)).await?;

        // extract exif data
        let mut exif_data = vec![0u8; exif_ptr.length - 4];
        guard.read_exact(&mut exif_data).await.map_err(|e| {
            anyhow!(
                "Failed to read exif (offset: {}, len: {}): {}",
                exif_ptr.offset,
                exif_ptr.length,
                e.to_string()
            )
        })?;

        // check exif data starts with "Exif\0\0"
        if &exif_data[0..6] != b"Exif\0\0" {
            return Err(anyhow!("Invalid exif data: not started with 'Exif'"));
        }

        // remove "Exif\0\0" from the beginning
        exif_data.drain(0..6);

        Ok(Some(exif_data))
    }
}

#[cfg(test)]
mod tests {
    use crate::{heic, internal::init_logger, ExtractRawExif};

    const SAMPLES: [&str; 2] = [
        "sample/sample_by_iphone15-pro-max.heic",
        "sample/sample_by_hasselblad-x2d.heic",
    ];

    #[tokio::test]
    async fn extract_exif_from_heic() {
        // init logger
        init_logger();

        // extract exif from samples
        for file in SAMPLES.into_iter() {
            let heic = heic(file).await.expect("Failed to open file");
            let exif_data = heic
                .extract()
                .await
                .expect("Failed to extract exif")
                .expect("Exif data must be existed");

            println!(
                "extracted exif from '{}' (exif length: {})\n{}",
                file,
                exif_data.len(),
                hex::encode(&exif_data)
            );
        }
    }
}
