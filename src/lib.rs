use anyhow::Result;
use async_trait::async_trait;

mod heic;

#[async_trait]
pub trait ManipulateExif {
    async fn raw_data(&self) -> Result<Vec<u8>>;
    async fn rates(&self) -> Result<i16>;
    async fn gps_info(&self) -> Result<(f64, f64)>;
    async fn add_gps_info(&mut self, lat: f64, lon: f64) -> Result<()>;
}
