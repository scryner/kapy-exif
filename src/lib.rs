use anyhow::Result;
use async_trait::async_trait;

mod heic;

#[async_trait]
pub trait ManipulateRawExif {
    async fn extract(&self) -> Result<Vec<u8>>;
    async fn replace(&mut self, data: &[u8]) -> Result<()>;
}
