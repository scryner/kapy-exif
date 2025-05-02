use std::io::{Cursor, SeekFrom};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::debug;
use tokio::io::{AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufWriter};

use crate::heic::Heic;
use crate::CopyWithRawExif;

use super::{ExtentValue, ItemLocationPtr, MetaBox};

enum Source<'a> {
    OriginFile(ItemLocationPtr),
    Data(&'a [u8]),
    Zero(u64),
}

fn sources_len(sources: &[Source]) -> usize {
    sources
        .iter()
        .map(|source| match source {
            Source::OriginFile(ptr) => ptr.ptr.length,
            Source::Data(data) => data.len(),
            Source::Zero(len) => *len as usize,
        })
        .sum()
}

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

        2. Extend mdat box
           - Adjust the size of the mdat box to accommodate the increased size of the EXIF data.
        */

        let exif = {
            let mut v = Vec::with_capacity(4 + 6 + exif.len());
            v.extend_from_slice(&vec![0, 0, 0, 6]); // 4 bytes
            v.extend_from_slice(b"Exif\0\0"); // 6 bytes
            v.extend_from_slice(exif); // exif.len() bytes
            v
        };

        // get metadata related information
        let exif_len = exif.len();

        let exif_item_id = self
            .full_box
            .meta
            .get_item_id("Exif")
            .ok_or(anyhow!("Exif item not found"))?;

        // make buffer to manipulate meta box
        let meta_len = self.full_box.meta.full_ptr.length;
        let mut meta_w = BufWriter::new(Cursor::new(vec![0u8; meta_len]));

        // copy original data to meta_w
        self.copy_with_scoped_ptr(&mut meta_w, &self.full_box.meta.full_ptr)
            .await?;

        // get extents to write as mdat box
        let sorted_extents = self
            .full_box
            .meta
            .iloc_box
            .as_ref()
            .ok_or(anyhow!("ItemLocationBox was not found"))? // actually never happened
            .sorted_extents();

        // define sources to how to write mdat box
        let mut mdat_sources = Vec::new();

        // modify metadata according to given exif
        let mut adjust_offset: u64 = 0;
        for (i, item) in sorted_extents.iter().enumerate() {
            if i != 0 {
                let prev_item = sorted_extents.get(i - 1).unwrap();
                let expected_offset = prev_item.ptr.offset + prev_item.ptr.length as u64;

                if expected_offset > item.ptr.offset {
                    return Err(anyhow!(
                        "Invalid mdat data: overlapped extents were existed"
                    ));
                } else if expected_offset < item.ptr.offset {
                    mdat_sources.push(Source::Zero(item.ptr.offset + expected_offset));
                }
            }

            if item.item_id == exif_item_id {
                let existed_exif_len = item.ptr.length;

                debug!(
                    "adjust exif iloc length: item_id({}/EXIF), length({}), adjusted_length({})",
                    item.item_id, existed_exif_len, exif_len,
                );

                // modify iloc length
                modify_iloc_item(
                    &mut meta_w,
                    &self.full_box.meta,
                    ModifyIlocItemKey::Id(exif_item_id),
                    None,
                    Some(exif_len),
                )
                .await?;

                if exif_len > existed_exif_len {
                    adjust_offset += (exif_len - existed_exif_len) as u64;
                    mdat_sources.push(Source::Data(&exif));
                } else if exif_len < existed_exif_len {
                    let zeros = (existed_exif_len - exif_len) as u64;
                    mdat_sources.push(Source::Data(&exif));
                    mdat_sources.push(Source::Zero(zeros));
                } else {
                    mdat_sources.push(Source::Data(&exif));
                }
            } else {
                if adjust_offset > 0 {
                    debug!(
                        "adjust iloc offset: item_id({}), offset({}) + {} = {})",
                        item.item_id,
                        item.ptr.offset,
                        adjust_offset,
                        item.ptr.offset + adjust_offset,
                    );

                    // modify iloc offset
                    modify_iloc_item(
                        &mut meta_w,
                        &self.full_box.meta,
                        ModifyIlocItemKey::Id(item.item_id),
                        Some(item.ptr.offset + adjust_offset),
                        None,
                    )
                    .await?;
                }

                mdat_sources.push(Source::OriginFile(item.clone()));
            }
        }

        // write ftyp
        self.copy_with_scoped_ptr(&mut writer, &self.full_box.ftyp.full_ptr)
            .await?;

        // get meta data from buffer
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
        write_mdat(&mut writer, &self, &mdat_sources).await?;

        Ok(())
    }
}

enum ModifyIlocItemKey<'a> {
    Id(u32),
    #[allow(unused)]
    Type(&'a str),
}

async fn modify_iloc_item<'a>(
    mut buf: impl AsyncWrite + AsyncSeek + Send + Sync + Unpin,
    meta: &MetaBox,
    key: ModifyIlocItemKey<'a>,
    offset: Option<u64>,
    length: Option<usize>,
) -> Result<()> {
    // get iloc box
    let iloc_box = meta
        .iloc_box
        .as_ref()
        .ok_or(anyhow!("ItemLocationBox was not found"))?;

    // get iloc entry
    let item_id = {
        match key {
            ModifyIlocItemKey::Id(item_id) => item_id,
            ModifyIlocItemKey::Type(item_type) => meta
                .get_item_id(item_type)
                .ok_or(anyhow!("Item '{}' not found", item_type))?,
        }
    };

    let entry = iloc_box
        .entries
        .get(&item_id)
        .ok_or(anyhow!("Extent not found"))?;

    // get extents
    if entry.extents.len() != 1 {
        return Err(anyhow!(
            "Only single extent is supported, but got {}",
            entry.extents.len()
        ));
    }

    let extent = &entry.extents[0]; // we already checked that there is only one extent

    // overwrite offset
    if let Some(offset) = offset {
        overwrite_extent(&mut buf, &extent.extent_offset, offset).await?;
    }

    // overwrite length
    if let Some(length) = length {
        overwrite_extent(&mut buf, &extent.extent_length, length as u64).await?;
    }

    Ok(())
}

async fn overwrite_extent(
    mut buf: impl AsyncWrite + AsyncSeek + Send + Sync + Unpin,
    extent_val: &ExtentValue,
    overwrite_val: u64,
) -> Result<()> {
    // seek to offset to according item
    buf.seek(SeekFrom::Start(extent_val.position())).await?;
    debug!("Seeked to position: {}", extent_val.position());

    // overwrite
    match extent_val {
        ExtentValue::U8(..) => buf.write_u8(overwrite_val as u8).await?,
        ExtentValue::U16(..) => buf.write_u16(overwrite_val as u16).await?,
        ExtentValue::U32(..) => buf.write_u32(overwrite_val as u32).await?,
        ExtentValue::U64(..) => buf.write_u64(overwrite_val as u64).await?,
    }

    Ok(())
}

async fn write_mdat<'a>(
    w: &mut (impl AsyncWrite + Send + Sync + Unpin),
    heic: &Heic,
    sources: &Vec<Source<'a>>,
) -> Result<()> {
    // write box header
    let mdat_len = sources_len(sources);
    debug!(
        "orig mdata length: {} / write mdat length: {}",
        heic.full_box.media.full_ptr.length,
        mdat_len + 16,
    );

    // write length (4 bytes with b0x01)
    w.write_u32(0x01).await?;

    // write mdat string
    w.write_all(b"mdat").await?;

    // write actual length
    w.write_u64(mdat_len as u64 + 16).await?; // 16 bytes: length + "mdat" + actual length

    // write box content
    for src in sources.iter() {
        match src {
            Source::OriginFile(item_location_ptr) => {
                debug!(
                    "copying from origin file: item_id({}), offset({}), length({})",
                    item_location_ptr.item_id,
                    item_location_ptr.ptr.offset,
                    item_location_ptr.ptr.length
                );
                heic.copy_with_scoped_ptr(w, &item_location_ptr.ptr).await?;
            }
            Source::Data(data) => {
                debug!("copying from data: length({})", data.len());
                w.write_all(data).await?;
            }
            Source::Zero(size) => {
                debug!("copying zero bytes: length({})", size);
                w.write_all(&vec![0; *size as usize]).await?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::SeekFrom;

    use tokio::{fs::File, io::AsyncSeekExt};

    use crate::{
        heic::{heic, FullBox, Heic},
        internal::{compare_files, init_logger},
        CopyWithRawExif, ExtractRawExif,
    };

    const SAMPLES: [&str; 1] = [
        // "sample/sample_by_iphone15-pro-max.heic",
        "sample/sample_by_hasselblad-x2d.heic",
    ];

    #[tokio::test]
    async fn copy_with_raw_exif_for_heic() {
        // init logger
        init_logger();

        // copy with exif (consequently, copy exactly the same file)
        for file in SAMPLES.iter() {
            // make tempfile
            let tmp = tempfile::tempfile().expect("Failed to create tempfile");
            let mut tmp = File::from_std(tmp);

            // copy with exif
            {
                let heic = heic(file).await.expect("Failed to open file");

                // extract exif_data
                let exif_data = heic
                    .extract()
                    .await
                    .expect("Failed to extract exif data")
                    .expect("Exif not found");

                // copy with raw exif
                heic.copy_with_raw_exif(&exif_data, &mut tmp)
                    .await
                    .expect("Failed to copy with raw exif");
            }

            // re-open the original file
            let mut orig = File::open(file).await.expect("Failed to open file");

            // compare the original file and the copied file
            assert!(compare_files(&mut orig, &mut tmp)
                .await
                .expect("Failed to compare files"));
        }
    }

    #[tokio::test]
    async fn copy_with_extended_exif_for_heic() {
        // init logger
        init_logger();

        const APPENDING_ZEROS: usize = 1234;

        // copy with exif (consequently, copy exactly the same file)
        for file in SAMPLES.iter() {
            // make tempfile
            let tmp = tempfile::tempfile().expect("Failed to create tempfile");
            let mut tmp = File::from_std(tmp);

            // copy with exif
            let (mdat_length, exif_length) = {
                let heic = Heic::from_file_path(file)
                    .await
                    .expect("Failed to open file");
                let expected_mdat_length = heic.full_box.media.full_ptr.length + APPENDING_ZEROS;

                // extract exif_data
                let mut exif_data = heic
                    .extract()
                    .await
                    .expect("Failed to extract exif data")
                    .expect("Exif not found");

                // append some data into exif data: must be valid exif data
                exif_data.extend_from_slice(&vec![0u8; 1234]);
                let expected_exif_length = exif_data.len() + 10; // "\0\0\0\0X6" + "Exif\0\0" = 10 bytes

                // copy with raw exif
                heic.copy_with_raw_exif(&exif_data, &mut tmp)
                    .await
                    .expect("Failed to copy with raw exif");

                (expected_mdat_length, expected_exif_length)
            };

            // seek file to 0
            tmp.seek(SeekFrom::Start(0))
                .await
                .expect("failed to seek file");

            // read full box
            let full_box = FullBox::from_reader(&mut tmp)
                .await
                .expect("failed to decode full box");

            // check mdat length
            assert_eq!(mdat_length, full_box.media.full_ptr.length);

            // retrieve exif length from meta
            let retrieved_exif_length = {
                let item = full_box.meta.get_item("Exif").expect("Exif not found");
                assert_eq!(item.len(), 1);
                let extent = &item[0].1;
                extent.extent_length.value() as usize
            };

            // check exif length
            assert_eq!(exif_length, retrieved_exif_length);
        }
    }
}
