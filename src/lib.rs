use anyhow::Result;
use async_trait::async_trait;

pub mod heic;

#[async_trait]
pub trait ExtractRawExif {
    async fn extract(&self) -> Result<Vec<u8>>;
}
