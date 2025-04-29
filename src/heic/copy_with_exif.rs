use std::io::{Cursor, SeekFrom};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::io::{AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufWriter};

use crate::heic::Heic;
use crate::CopyWithRawExif;

use super::ExtentValue;

#[async_trait]
impl CopyWithRawExif for Heic {
    async fn copy_with_raw_exif(
        &self,
        exif: &[u8],
        mut writer: impl AsyncWrite + Send + Sync + Unpin,
    ) -> Result<()> {
        /*
        Strategy to replace the given EXIF data in the image's EXIF information:

        1. In-place update
           - If the size of the given EXIF data is smaller than the existing EXIF data, overwrite the existing EXIF data.
           - In this case, the remaining space will be filled with garbage values, but it is expected to be harmless.

        2. Use free space
           - If a free box exists, utilize it.
           - Adjust the starting position of the mdat box to be inside the free box to store the added data.

        3. Extend mdat box
           - Adjust the size of the mdat box to accommodate the increased size of the EXIF data.
        */

        // get several lengths
        let exif_len = exif.len();
        let free_len = {
            match &self.full_box.free {
                Some(free) => free.full_ptr.length,
                None => 0,
            }
        };
        let (exif_item_id, prev_exif_len, prev_exif_extent) = {
            let (id, (exif_ptr, exif_extent)) =
                self.exif_ptr().ok_or(anyhow!("Exif item not found"))?;
            (id, exif_ptr.length, exif_extent)
        };

        // change new_meta according to strategies above commented
        if prev_exif_len > exif_len {
            /* in-place update: just change exif length */

            // make buffer to manipulate meta
            let meta_len = self.full_box.meta.full_ptr.length;
            let mut meta_w = BufWriter::new(Cursor::new(vec![0u8; meta_len]));

            // copy original data to meta_w
            self.copy_with_scoped_ptr(&mut meta_w, &self.full_box.meta.full_ptr)
                .await?;

            // seek to offset pointing to exif length
            meta_w
                .seek(SeekFrom::Start(prev_exif_extent.extent_length.position()))
                .await?;

            match prev_exif_extent.extent_length {
                ExtentValue::U8(..) => meta_w.write_u8(exif_len as u8).await?,
                ExtentValue::U16(..) => meta_w.write_u16(exif_len as u16).await?,
                ExtentValue::U32(..) => meta_w.write_u32(exif_len as u32).await?,
                ExtentValue::U64(..) => meta_w.write_u64(exif_len as u64).await?,
            }

            // write ftyp
            self.copy_with_scoped_ptr(&mut writer, &self.full_box.ftyp.full_ptr)
                .await?;

            // get meta data
            let meta_data = meta_w.into_inner().into_inner();

            // write meta
            writer.write_all(&meta_data).await?;

            // write free
            if let Some(free) = self.full_box.free.as_ref() {
                // write free box
                self.copy_with_scoped_ptr(&mut writer, &free.full_ptr)
                    .await?;
            }

            // write mdat
            let sorted_extents = self
                .full_box
                .meta
                .iloc_box
                .as_ref()
                .ok_or(anyhow!("ItemLocationBox was not found"))?
                .sorted_extents();

            self.write_mdat_with_replacing_item(&mut writer, &sorted_extents, exif_item_id, exif)
                .await?;

            Ok(())
        } else if (exif_len - prev_exif_len) < free_len {
            // use free space
            todo!()
        } else {
            // extend mdat box
            todo!()
        }
    }
}
