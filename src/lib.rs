use anyhow::Result;
use async_trait::async_trait;

mod heic;
mod internal;
mod jpeg;

pub use heic::heic;
pub use jpeg::jpeg;

use tokio::io::AsyncWrite;

#[async_trait]
pub trait ExtractRawExif {
    async fn extract(&self) -> Result<Vec<u8>>;
}

#[async_trait]
pub trait CopyWithRawExif {
    async fn copy_with_raw_exif(
        &self,
        exif: Vec<u8>,
        writer: impl AsyncWrite + Send + Sync,
    ) -> Result<()>;
}
