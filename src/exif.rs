use std::ffi::{c_char, c_int, CStr, CString};

use anyhow::{anyhow, Result};

#[repr(C)]
struct ExifMetadataT {
    // opaque structure
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[link(name = "libexif")]
extern "C" {
    fn exif_metadata_new() -> *mut ExifMetadataT;
    fn exif_metadata_from_blob(
        metadata: *mut ExifMetadataT,
        blob: *const u8,
        blob_len: usize,
    ) -> c_int;
    fn exif_metadata_to_blob(metadata: *mut ExifMetadataT, blob: *mut *mut u8) -> usize;
    fn exif_get_tag_string(metadata: *mut ExifMetadataT, tag: *const c_char) -> *mut c_char;
    fn exif_metadata_destroy(metadata: *const *mut ExifMetadataT);
    fn exif_metadata_add_gps_info(
        metadata: *mut ExifMetadataT,
        lat: f64,
        lon: f64,
        alt: f64,
    ) -> c_int;
}

// safe implementation
pub struct Metadata {
    raw: *mut ExifMetadataT,
}

impl Drop for Metadata {
    fn drop(&mut self) {
        unsafe {
            exif_metadata_destroy(&self.raw);
        }
    }
}

impl Metadata {
    pub fn new_from_exif_blob(blob: &Vec<u8>) -> Result<Self> {
        unsafe {
            let raw = exif_metadata_new();
            let blob_len = blob.len();
            let blob_ptr = blob.as_ptr();

            if exif_metadata_from_blob(raw, blob_ptr, blob_len) != 0 {
                return Err(anyhow!("Failed to create metadata from image blob"));
            }

            Ok(Metadata { raw })
        }
    }

    pub fn get_tag<T>(&self, tag: T) -> Option<String>
    where
        T: AsRef<str>,
    {
        let tag = CString::new(tag.as_ref()).unwrap();
        let tag = tag.as_ptr();

        unsafe {
            let val = exif_get_tag_string(self.raw, tag);
            if val.is_null() {
                return None;
            }

            let val = CStr::from_ptr(val as *const c_char);
            let val = val.to_str().unwrap().to_string();

            Some(val)
        }
    }

    pub fn dump(&self) -> Result<Vec<u8>> {
        unsafe {
            let mut blob: *mut u8 = std::ptr::null_mut();
            let blob_len = exif_metadata_to_blob(self.raw, &mut blob);

            if blob.is_null() || blob_len <= 0 {
                return Err(anyhow!("Failed to dump metadata to blob"));
            }

            Ok(Vec::from_raw_parts(blob, blob_len, blob_len))
        }
    }

    pub fn update_gps_info(&mut self, lat: f64, lon: f64, alt: f64) -> Result<()> {
        unsafe {
            if exif_metadata_add_gps_info(self.raw, lat, lon, alt) != 0 {
                return Err(anyhow!("Failed to update GPS info"));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{exif::Metadata, heic, jpeg, CopyWithRawExif, ExtractRawExif};

    const SAMPLES: [&str; 3] = [
        "sample/sample_by_pentax-k1.jpg",
        "sample/sample_by_iphone15-pro-max.heic",
        "sample/sample_by_hasselblad-x2d.heic",
    ];

    #[tokio::test]
    async fn exif_blob() {
        for file in SAMPLES.into_iter() {
            let exif_data = if file.ends_with(".jpg") {
                jpeg(file)
                    .await
                    .expect("Failed to read JPEG file")
                    .extract()
                    .await
                    .expect("Failed to extract EXIF data")
                    .expect("No EXIF data found")
            } else if file.ends_with("heic") {
                heic(file)
                    .await
                    .expect("Failed to read HEIC file")
                    .extract()
                    .await
                    .expect("Failed to extract EXIF data")
                    .expect("No EXIF data found")
            } else {
                // never happened
                panic!("Unsupported file type");
            };

            println!("{}: exif_len({})", file, exif_data.len());

            // read EXIF data from blob
            let mut metadata =
                Metadata::new_from_exif_blob(&exif_data).expect("Failed to create metadata");

            let camera_model = metadata
                .get_tag("Exif.Image.Model")
                .expect("Failed to get tag");

            println!("Camera Model: {}", camera_model);

            // dump EXIF data to blob
            let dumped = metadata.dump().expect("Failed to dump metadata");
            let dumped_camera_model = Metadata::new_from_exif_blob(&dumped)
                .expect("Failed to create metadata")
                .get_tag("Exif.Image.Model")
                .expect("Failed to get tag");

            assert_eq!(camera_model, dumped_camera_model);

            // update GPS info
            let lat = 37.7749;
            let lon = -122.4194;
            let alt = 10.0;

            metadata
                .update_gps_info(lat, lon, alt)
                .expect("Failed to update GPS info");

            // get updated GPS info
            let gps_lat = metadata
                .get_tag("Exif.GPSInfo.GPSLatitude")
                .expect("Failed to get GPS latitude");

            assert_eq!(gps_lat, String::from("37/1 46/1 29640000/1000000"))
        }
    }

    #[tokio::test]
    async fn update_gps_info_for_heic() {
        let file = "sample/sample_by_hasselblad-x2d.heic";
        let image = heic(file).await.expect("Failed to read HEIC file");

        // read EXIF data from blob
        let mut metadata = {
            let blob = image
                .extract()
                .await
                .expect("Failed to extract EXIF data")
                .expect("No EXIF data found");

            Metadata::new_from_exif_blob(&blob).expect("Failed to create metadata")
        };

        // update GPS info
        let lat = 37.7749;
        let lon = -122.4194;
        let alt = 10.0;

        metadata
            .update_gps_info(lat, lon, alt)
            .expect("Failed to update GPS info");

        // dump EXIF data to blob
        let dumped = metadata.dump().expect("Failed to dump metadata");

        // make a file
        let mut out = tokio::fs::File::create("reconstructed.heic")
            .await
            .expect("Failed to create file");

        image
            .copy_with_raw_exif(&dumped, &mut out)
            .await
            .expect("Failed to copy EXIF data");
    }
}
