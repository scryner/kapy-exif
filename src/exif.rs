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
    fn exif_get_last_error() -> *const c_char;
    fn exif_clear_last_error();

    #[allow(unused)] // only used for testing
    fn exif_test_error_handling(expected: *const c_char) -> c_int;
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
                let err_msg = get_last_error()
                    .unwrap_or_else(|| "Unknown error creating metata from blob".to_string());
                return Err(anyhow!(err_msg));
            }

            Ok(Metadata { raw })
        }
    }

    pub fn get_tag<T>(&self, tag: T) -> Result<Option<String>>
    where
        T: AsRef<str>,
    {
        let tag = CString::new(tag.as_ref()).unwrap();
        let tag = tag.as_ptr();

        unsafe {
            let val = exif_get_tag_string(self.raw, tag);
            if val.is_null() {
                match get_last_error() {
                    Some(err_msg) => return Err(anyhow!(err_msg)),
                    None => return Ok(None),
                }
            }

            let val = CStr::from_ptr(val as *const c_char);
            let val = val.to_str().unwrap().to_string();

            Ok(Some(val))
        }
    }

    pub fn dump(&self) -> Result<Vec<u8>> {
        unsafe {
            let mut blob: *mut u8 = std::ptr::null_mut();
            let blob_len = exif_metadata_to_blob(self.raw, &mut blob);

            if blob.is_null() || blob_len <= 0 {
                let err_msg = get_last_error()
                    .unwrap_or_else(|| "Unknown error dumping metadata to blob".to_string());

                return Err(anyhow!(err_msg));
            }

            Ok(Vec::from_raw_parts(blob, blob_len, blob_len))
        }
    }

    pub fn update_gps_info(&mut self, lat: f64, lon: f64, alt: f64) -> Result<()> {
        unsafe {
            if exif_metadata_add_gps_info(self.raw, lat, lon, alt) != 0 {
                let err_msg = get_last_error()
                    .unwrap_or_else(|| "Unknown error updating GPS info".to_string());

                return Err(anyhow!(err_msg));
            }
        }

        Ok(())
    }
}

// helper function to get last error message
fn get_last_error() -> Option<String> {
    unsafe {
        let error_ptr = exif_get_last_error();
        if error_ptr.is_null() {
            return None;
        }

        let c_str = CStr::from_ptr(error_ptr);
        match c_str.to_str() {
            Ok(s) => {
                let res = s.to_string();

                // clear error message
                exif_clear_last_error();

                // return error message
                Some(res)
            }
            Err(_) => Some("Invalid error message encoding".to_string()),
        }
    }
}

// helper function to test error handling
#[allow(unused)]
#[cfg(test)]
fn test_error_handling(expected: impl AsRef<str>) -> Result<String> {
    let expected = CString::new(expected.as_ref()).unwrap();
    let expected = expected.as_ptr();

    unsafe {
        let res = exif_test_error_handling(expected);
        if res < 0 {
            match get_last_error() {
                Some(err_msg) => Ok(err_msg),
                None => Err(anyhow!("Error message not found")),
            }
        } else {
            Err(anyhow!("Unexpected success"))
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::fs::File;

    use crate::{
        exif::test_error_handling,
        exif::Metadata,
        heic::{heic, Heic},
        internal::init_logger,
        jpeg, CopyWithRawExif, ExtractRawExif,
    };

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
                .expect("Failed to get tag")
                .expect("Must have the metadata");

            println!("Camera Model: {}", camera_model);

            // dump EXIF data to blob
            let dumped = metadata.dump().expect("Failed to dump metadata");
            let dumped_camera_model = Metadata::new_from_exif_blob(&dumped)
                .expect("Failed to create metadata")
                .get_tag("Exif.Image.Model")
                .expect("Failed to get tag")
                .expect("Must have the metadata");

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
                .expect("Failed to get GPS latitude")
                .expect("Must have the metadata");

            assert_eq!(gps_lat, String::from("37/1 46/1 29640000/1000000"));
        }
    }

    #[tokio::test]
    async fn update_gps_info_for_heic() {
        init_logger();

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
        let tmp = tempfile::NamedTempFile::new().expect("Failed to create temporary file");
        let mut tmp = File::from_std(tmp.into_file());

        // copy with injected gps
        image
            .copy_with_raw_exif(&dumped, &mut tmp)
            .await
            .expect("Failed to copy EXIF data");

        // make HEIC struct from written
        let gps_added = Heic::from_file(tmp)
            .await
            .expect("Failed to read HEIC file");

        let gps_added_exif_data = gps_added
            .extract()
            .await
            .expect("Failed to extract EXIF data")
            .expect("No EXIF data found");

        let gps_added_metadata =
            Metadata::new_from_exif_blob(&gps_added_exif_data).expect("Failed to create metadata");

        let gps_lat = gps_added_metadata
            .get_tag("Exif.GPSInfo.GPSLatitude")
            .expect("Failed to get GPS latitude")
            .expect("Must have the metadata");

        assert_eq!(gps_lat, String::from("37/1 46/1 29640000/1000000"));
    }

    #[test]
    fn error_handling() {
        let expected = vec!["Test Messsage 1", "Test Message 2"];

        for msg in expected.iter() {
            let res = test_error_handling(msg).expect("Failed to handle error");
            assert_eq!(res, msg.to_string());
        }
    }
}
