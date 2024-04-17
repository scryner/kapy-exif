use anyhow::Result;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncSeek};

use crate::ManipulateExif;

pub struct Heic<R> {
    r: R,
}

impl<R> Heic<R>
where
    R: AsyncRead + AsyncSeek + Send + Sync,
{
    pub fn new(r: R) -> Self {
        Self { r }
    }
}

#[async_trait]
impl<R> ManipulateExif for Heic<R>
where
    R: AsyncRead + AsyncSeek + Send + Sync,
{
    async fn raw_data(&self) -> Result<Vec<u8>> {
        unimplemented!()
    }

    async fn rates(&self) -> Result<i16> {
        unimplemented!()
    }

    async fn gps_info(&self) -> Result<(f64, f64)> {
        unimplemented!()
    }

    async fn add_gps_info(&mut self, _lat: f64, _lon: f64) -> Result<()> {
        unimplemented!()
    }
}
