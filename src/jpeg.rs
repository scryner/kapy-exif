use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{path::Path, sync::Arc};

use tokio::{fs::File, io::AsyncWrite, sync::Mutex};

use crate::{CopyWithRawExif, ExtractRawExif};

pub fn jpeg(path: impl AsRef<Path>) -> Result<impl ExtractRawExif> {
    // open file
    let file =
        std::fs::File::open(path.as_ref()).map_err(|e| anyhow!("Failed to open file: {}", e))?;

    Ok(Jpeg {
        file: Arc::new(Mutex::new(File::from_std(file))),
    })
}

struct Jpeg {
    file: Arc<Mutex<File>>,
}

#[async_trait]
impl ExtractRawExif for Jpeg {
    async fn extract(&self) -> Result<Vec<u8>> {
        todo!()
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
