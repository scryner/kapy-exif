use std::{
    collections::HashMap,
    io::{Cursor, ErrorKind, SeekFrom},
    path::Path,
    sync::Arc,
    u8,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::debug;
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite},
    sync::Mutex,
};

use crate::{CopyWithRawExif, ExtractRawExif};

pub async fn heic(path: impl AsRef<Path>) -> Result<impl ExtractRawExif> {
    // open file
    let mut file = File::open(path.as_ref())
        .await
        .map_err(|e| anyhow!("Failed to open file: {}", e))?;

    // decode full box
    let full_box = FullBox::from_reader(&mut file).await?;

    // seek start position for file
    file.seek(SeekFrom::Start(0))
        .await
        .map_err(|e| anyhow!("Failed to seek: {}", e))?;

    Ok(Heic {
        file: Arc::new(Mutex::new(file)),
        full_box,
    })
}

struct Heic {
    file: Arc<Mutex<File>>,
    full_box: FullBox,
}

#[async_trait]
impl ExtractRawExif for Heic {
    async fn extract(&self) -> Result<Option<Vec<u8>>> {
        // get offset and length of exif box
        let exif_ptrs = match self.full_box.meta.get_item_ptr("Exif") {
            Some(ptr) => ptr,
            None => return Ok(None),
        };

        let exif_ptr = exif_ptrs.first().ok_or(anyhow!("Exif extent not found"))?;
        debug!("exif ptr: {:?}", exif_ptr);

        // check exif length to avoid OOM (maybe, due to malicious file or uncaught errors on parsing ISOBMFF format)
        const MAX_LEN: usize = 1024 * 1024 * 8; // 8MB

        if exif_ptr.length - 4 > MAX_LEN {
            // I don't know why -4 is needed currently (it may be another header)
            return Err(anyhow!("Exif length is too large: {}", exif_ptr.length - 4));
        }

        // read exif content from file
        let mut guard = self.file.lock().await;

        // seek to raw exif content
        guard.seek(SeekFrom::Start(exif_ptr.offset + 4)).await?;

        // extract exif data
        let mut exif_data = vec![0u8; exif_ptr.length - 4];
        guard.read_exact(&mut exif_data).await.map_err(|e| {
            anyhow!(
                "Failed to read exif (offset: {}, len: {}): {}",
                exif_ptr.offset,
                exif_ptr.length,
                e.to_string()
            )
        })?;

        // check exif data starts with "Exif"
        if &exif_data[0..4] != b"Exif" {
            return Err(anyhow!("Invalid exif data: not started with 'Exif'"));
        }

        Ok(Some(exif_data))
    }
}

#[async_trait]
impl CopyWithRawExif for Heic {
    async fn copy_with_raw_exif(
        &self,
        exif: &[u8],
        writer: impl AsyncWrite + Send + Sync,
    ) -> Result<()> {
        todo!()
    }
}

#[derive(Clone, Debug)]
struct Ptr {
    offset: u64,
    length: usize,
}

impl Ptr {
    async fn read_data<R>(&self, r: &mut R) -> Result<Vec<u8>>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        // seek to offset
        r.seek(SeekFrom::Start(self.offset)).await?;

        // read data
        let mut buf = vec![0u8; self.length];
        r.read_exact(&mut buf).await?;

        Ok(buf)
    }
}

#[derive(Debug)]
struct RawBox {
    box_type: String,
    full_ptr: Ptr,
    data_ptr: Ptr,
}

impl RawBox {
    async fn from_reader<R>(r: &mut R) -> Result<Option<Self>>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        // get current offset
        let offset = r.seek(std::io::SeekFrom::Current(0)).await?;

        // read first 4 bytes from reader
        let length = {
            let mut buf = [0u8; 4];
            match r.read_exact(&mut buf).await {
                Ok(_) => (),
                Err(e) => {
                    // return None if EOF
                    if e.kind() == ErrorKind::UnexpectedEof {
                        return Ok(None);
                    } else {
                        return Err(e.into());
                    }
                }
            }

            // convert to u32 in big endian
            u32::from_be_bytes(buf) as usize
        };

        // read next 4 bytes from reader to get box type
        let box_type = {
            let mut buf = [0u8; 4];
            r.read_exact(&mut buf).await?;

            // convert to string
            String::from_utf8(buf.to_vec())?
        };

        let (full_ptr, data_ptr) = match length {
            1 => {
                let mut buf = [0u8; 8];
                r.read_exact(&mut buf).await?;

                // convert to u64 in big endian
                let length = u64::from_be_bytes(buf) as usize;

                let full_ptr = Ptr { offset, length };

                let data_ptr = Ptr {
                    offset: offset + 16,
                    length: length - 16,
                };

                (full_ptr, data_ptr)
            }
            _ => {
                let full_ptr = Ptr { offset, length };

                let data_ptr = Ptr {
                    offset: offset + 8,
                    length: length - 8,
                };

                (full_ptr, data_ptr)
            }
        };

        Ok(Some(Self {
            box_type,
            full_ptr,
            data_ptr,
        }))
    }

    async fn advance<R>(&self, r: &mut R) -> Result<()>
    where
        R: AsyncSeek + Send + Sync + Unpin,
    {
        let next_offset = self.full_ptr.offset + self.full_ptr.length as u64;

        // seek
        r.seek(SeekFrom::Start(next_offset)).await?;

        Ok(())
    }
}

#[allow(unused)]
#[derive(Debug)]
struct FullBox {
    pub(crate) ftyp: FileTypeBox,    // ftyp
    pub(crate) meta: MetaBox,        // meta
    pub(crate) free: Option<RawBox>, // free
    pub(crate) media: RawBox,        // mdat
}

impl FullBox {
    async fn from_reader<R>(r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        let mut ftyp: Option<FileTypeBox> = None;
        let mut meta: Option<MetaBox> = None;
        let mut free: Option<RawBox> = None;
        let mut media: Option<RawBox> = None;

        loop {
            // read raw box
            let raw_box = match RawBox::from_reader(r).await? {
                Some(b) => b,
                None => break,
            };

            let box_type = raw_box.box_type.as_str();
            debug!("box entry: {} / {:?}", box_type, raw_box);

            match box_type {
                "ftyp" => {
                    let file_type_box = FileTypeBox::from_raw_box(&raw_box, r).await?;
                    ftyp = Some(file_type_box);
                }
                "meta" => {
                    let meta_box = MetaBox::from_raw_box(&raw_box, r).await?;
                    meta = Some(meta_box);
                }
                "free" => {
                    raw_box.advance(r).await?;
                    free = Some(raw_box);
                }
                "mdat" => {
                    raw_box.advance(r).await?;
                    media = Some(raw_box);
                }
                _ => return Err(anyhow::anyhow!("Unknown box type: {}", box_type)),
            }
        }

        if ftyp.is_none() || meta.is_none() || media.is_none() {
            return Err(anyhow::anyhow!("Missing required boxes"));
        }

        Ok(Self {
            ftyp: ftyp.unwrap(),
            meta: meta.unwrap(),
            free,
            media: media.unwrap(),
        })
    }
}

#[allow(unused)]
#[derive(Debug)]
struct FileTypeBox {
    pub(crate) major_brand: String,
    pub(crate) minor_version: u32,
    pub(crate) compatible_brands: Vec<String>,

    pub(crate) data_ptr: Ptr,
}

impl FileTypeBox {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        let data = raw_box.data_ptr.read_data(r).await?;

        // check format
        if data.len() < 8 {
            return Err(anyhow::anyhow!("Invalid data length: less than 8 bytes"));
        }

        if data.len() % 4 != 0 {
            return Err(anyhow::anyhow!(
                "Invalid data length: not multiple of 4 bytes"
            ));
        }

        // read major brand
        let major_brand = &data[0..4];
        let major_brand = String::from_utf8_lossy(major_brand).to_string();

        // read minor brand
        let minor_brand = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);

        // read compatible brands
        let mut compatible_brands = Vec::new();
        for i in (8..data.len()).step_by(4) {
            let brand = &data[i..i + 4];
            let brand = String::from_utf8_lossy(brand).to_string();

            compatible_brands.push(brand);
        }

        Ok(Self {
            major_brand,
            minor_version: minor_brand,
            compatible_brands,
            data_ptr: raw_box.data_ptr.clone(),
        })
    }
}

#[allow(unused)]
#[derive(Debug)]
struct MetaBox {
    pub(crate) version: u8,
    pub(crate) flags: u32,
    pub(crate) boxes: HashMap<String, RawBox>,

    pub(crate) iinf_box: Option<ItemInfoBox>,
    pub(crate) iloc_box: Option<ItemLocationBox>,

    pub(crate) full_ptr: Ptr,
    pub(crate) data_ptr: Ptr,
}

impl MetaBox {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        let data = raw_box.data_ptr.read_data(r).await?;

        // read version
        let (version, flags) = get_version_and_flags(&data)?;

        // get offset
        let offset = raw_box.data_ptr.offset + 4;

        // read children boxes
        let mut cursor = std::io::Cursor::new(&data[4..]);

        let mut iinf_box: Option<ItemInfoBox> = None;
        let mut iloc_box: Option<ItemLocationBox> = None;

        let mut boxes = HashMap::new();
        loop {
            let mut raw_box = match RawBox::from_reader(&mut cursor).await? {
                Some(b) => b,
                None => break,
            };

            match raw_box.box_type.as_str() {
                "iinf" => {
                    let iinf = ItemInfoBox::from_raw_box(&raw_box, &mut cursor).await?;
                    iinf_box = Some(iinf);
                }
                "iloc" => {
                    let iloc = ItemLocationBox::from_raw_box(&raw_box, &mut cursor).await?;
                    iloc_box = Some(iloc);
                }
                _ => {
                    // advance
                    raw_box.advance(&mut cursor).await?;
                }
            }

            // adjust offset
            raw_box.full_ptr.offset += offset;
            raw_box.data_ptr.offset += offset;

            boxes.insert(raw_box.box_type.clone(), raw_box);
        }

        Ok(Self {
            version,
            flags,
            boxes,
            iinf_box,
            iloc_box,
            full_ptr: raw_box.full_ptr.clone(),
            data_ptr: raw_box.data_ptr.clone(),
        })
    }

    fn get_item_ptr(&self, item_type: &str) -> Option<Vec<Ptr>> {
        // get item info box entry
        let iinf_entry = self
            .iinf_box
            .as_ref()?
            .entries
            .iter()
            .find(|e| e.item_type == item_type)?;

        let item_id = iinf_entry.item_id;

        // get item location entry
        let iloc_entry = self.iloc_box.as_ref()?.entries.get(&item_id)?;
        let mut ptrs = Vec::new();

        for extent in &iloc_entry.extents {
            let ptr = Ptr {
                offset: iloc_entry.base_offset + extent.extent_offset,
                length: extent.extent_length as usize,
            };

            ptrs.push(ptr);
        }

        Some(ptrs)
    }
}

fn get_version_and_flags(data: &[u8]) -> Result<(u8, u32)> {
    if data.len() < 4 {
        return Err(anyhow!("Invalid data length: less than 4 bytes"));
    }

    let version = data[0];
    let flags = u32::from_be_bytes([0, data[1], data[2], data[3]]);

    Ok((version, flags))
}

#[allow(unused)]
#[derive(Debug)]
struct ItemInfoBox {
    pub(crate) version: u8,
    pub(crate) flags: u32,

    pub(crate) entries: Vec<ItemInfoEntry>,

    pub(crate) full_ptr: Ptr,
    pub(crate) data_ptr: Ptr,
}

impl ItemInfoBox {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        let data = raw_box.data_ptr.read_data(r).await?;

        // check data length
        if data.len() < 6 {
            return Err(anyhow!("Invalid data length: less than 6 bytes"));
        }

        // read version and flags
        let (version, flags) = get_version_and_flags(&data)?;

        // read counts
        let counts: u16 = u16::from_be_bytes([data[4], data[5]]);

        // read entries
        let mut cursor = std::io::Cursor::new(&data[6..]);
        let mut entries = Vec::new();

        for _ in 0..counts {
            // make raw box
            let item_raw_box = RawBox::from_reader(&mut cursor)
                .await?
                .ok_or(anyhow!("EOF: must be existed"))?;

            let entry = ItemInfoEntry::from_raw_box(&item_raw_box, &mut cursor).await?;
            debug!("item info entry: {:?}", entry);

            entries.push(entry);
        }

        Ok(Self {
            version,
            flags,
            entries,
            full_ptr: raw_box.full_ptr.clone(),
            data_ptr: raw_box.data_ptr.clone(),
        })
    }
}

#[allow(unused)]
#[derive(Debug)]
struct ItemInfoEntry {
    pub(crate) version: u8,
    pub(crate) flags: u32,

    pub(crate) item_id: u32,
    pub(crate) protection_index: u16,

    pub(crate) item_type: String,
}

impl ItemInfoEntry {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        let data = raw_box.data_ptr.read_data(r).await?;
        if data.len() < 13 {
            return Err(anyhow!(
                "Invalid data length: must be greater than 13 bytes (actual: {})",
                data.len()
            ));
        }

        // read version and flags
        let (version, flags) = get_version_and_flags(&data)?;

        // check version
        if version != 2 {
            return Err(anyhow!(
                "Invalid version: {} (only version 2 was supported)",
                version
            ));
        }

        // read item id
        let item_id = u16::from_be_bytes([data[4], data[5]]) as u32;

        // read protection index
        let protection_index = u16::from_be_bytes([data[6], data[7]]);

        // read item type
        let item_type = String::from_utf8_lossy(&data[8..12]).to_string();

        Ok(Self {
            version,
            flags,
            item_id,
            protection_index,
            item_type,
        })
    }
}

#[allow(unused)]
#[derive(Debug)]
struct ItemLocationBox {
    pub(crate) version: u8,
    pub(crate) flags: u32,

    pub(crate) entries: HashMap<u32, ItemLocationEntry>,

    pub(crate) full_ptr: Ptr,
    pub(crate) data_ptr: Ptr,
}

#[allow(unused)]
#[derive(Debug)]
struct ItemLocationEntry {
    pub(crate) item_id: u32,
    pub(crate) construction_method: u16,
    pub(crate) data_reference_index: u16,
    pub(crate) base_offset: u64,
    pub(crate) extents: Vec<ItemLocationExtent>,
}

impl ItemLocationEntry {
    async fn from_cursor(
        r: &mut Cursor<&[u8]>,
        version: u8,
        offset_size: u8,
        length_size: u8,
        base_offset_size: u8,
        index_size: u8,
    ) -> Result<Self> {
        let item_id = match version {
            1 => r.read_u16().await? as u32,
            2 => r.read_u32().await?,
            _ => return Err(anyhow!("Invalid version: {}", version)),
        };

        let construction_method = {
            // read 4 bits (upper 12 bits are reserved)
            r.read_u16().await? & 0x0F
        };

        let data_reference_index = r.read_u16().await?;

        let base_offset = match base_offset_size {
            // in byte size
            0 => 0,
            1 => r.read_u8().await? as u64,
            2 => r.read_u16().await? as u64,
            4 => r.read_u32().await? as u64,
            8 => r.read_u64().await?,
            _ => return Err(anyhow!("Invalid base offset size: {}", base_offset_size)),
        };

        let extent_count = r.read_u16().await?;
        let mut extents = Vec::new();

        for _ in 0..extent_count {
            let extent_index = match index_size {
                0 => 0,
                1 => r.read_u8().await? as u64,
                2 => r.read_u16().await? as u64,
                4 => r.read_u32().await? as u64,
                8 => r.read_u64().await?,
                _ => return Err(anyhow!("Invalid index size: {}", index_size)),
            };

            let extent_offset = match offset_size {
                1 => r.read_u8().await? as u64,
                2 => r.read_u16().await? as u64,
                4 => r.read_u32().await? as u64,
                8 => r.read_u64().await?,
                _ => return Err(anyhow!("Invalid offset size: {}", offset_size)),
            };

            let extent_length = match length_size {
                1 => r.read_u8().await? as u64,
                2 => r.read_u16().await? as u64,
                4 => r.read_u32().await? as u64,
                8 => r.read_u64().await?,
                _ => return Err(anyhow!("Invalid length size: {}", length_size)),
            };

            extents.push(ItemLocationExtent {
                extent_index,
                extent_offset,
                extent_length,
            });
        }

        Ok(Self {
            item_id,
            construction_method,
            data_reference_index,
            base_offset,
            extents,
        })
    }
}

#[allow(unused)]
#[derive(Debug)]
struct ItemLocationExtent {
    extent_index: u64,
    extent_offset: u64,
    extent_length: u64,
}

impl ItemLocationBox {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        debug!("parsing iloc from {}", r.stream_position().await?);

        // read data
        let data = raw_box.data_ptr.read_data(r).await?;
        debug!("iloc data length: {}", data.len());
        debug!("iloc data: {}", hex::encode(&data));

        // read version and flags
        let (version, flags) = get_version_and_flags(&data)?;

        // make cursor
        let mut cursor = Cursor::new(&data[4..]);

        // read further 1 byte to indicate offset size and length size
        let (offset_size, length_size) = {
            let u8 = cursor.read_u8().await?;
            (u8 >> 4, u8 & 0x0F)
        };

        // read base offset size and index size
        let (base_offset_size, index_size) = {
            let u8 = cursor.read_u8().await?;
            (u8 >> 4, u8 & 0x0F)
        };

        // read count
        let count = match version {
            1 => cursor.read_u16().await? as usize,
            2 => cursor.read_u32().await? as usize,
            _ => return Err(anyhow!("Invalid version: {}", version)),
        };

        // make iloc entries
        let mut entries = HashMap::new();

        for _ in 0..count {
            let entry = ItemLocationEntry::from_cursor(
                &mut cursor,
                version,
                offset_size,
                length_size,
                base_offset_size,
                index_size,
            )
            .await?;

            debug!("iloc entry: {:?}", entry);

            entries.insert(entry.item_id, entry);
        }

        Ok(Self {
            version,
            flags,
            entries,
            full_ptr: raw_box.full_ptr.clone(),
            data_ptr: raw_box.data_ptr.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use tokio::fs;

    use crate::{
        heic::{heic, FullBox},
        internal::init_logger,
        ExtractRawExif,
    };

    const SAMPLES: [&str; 2] = [
        "sample/sample_by_iphone15-pro-max.heic",
        "sample/sample_by_hasselblad-x2d.heic",
    ];

    #[tokio::test]
    async fn read_full_box() {
        // init logger
        init_logger();

        // read full box from samples
        for file in SAMPLES.into_iter() {
            let mut f = fs::File::open(file).await.expect("Failed to open file");

            let full_box = FullBox::from_reader(&mut f)
                .await
                .expect("failed to read full box");

            println!("--------{}\n{:#?}", file, full_box);
        }
    }

    #[tokio::test]
    async fn extract_exif_from_heic() {
        // init logger
        init_logger();

        // extract exif from samples
        for file in SAMPLES.into_iter() {
            let heic = heic(file).await.expect("Failed to open file");
            let exif_data = heic
                .extract()
                .await
                .expect("Failed to extract exif")
                .expect("Exif data must be existed");

            println!(
                "--------{} (content length: {})\n{}",
                file,
                exif_data.len(),
                hex::encode(&exif_data)
            );
        }
    }
}
