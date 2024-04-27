use anyhow::Result;

pub struct Exif {
    // TODO: Implement this struct
}

impl Exif {
    pub fn new(data: Vec<u8>) -> Self {
        unimplemented!()
    }

    pub fn add_gps_info(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        unimplemented!()
    }

    pub fn data(&self) -> &[u8] {
        unimplemented!()
    }
}
