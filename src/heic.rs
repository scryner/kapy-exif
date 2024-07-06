use std::{
    collections::HashMap,
    io::{Cursor, ErrorKind, SeekFrom},
    u8,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::debug;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

use crate::ManipulateRawExif;

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
impl<R> ManipulateRawExif for Heic<R>
where
    R: AsyncRead + AsyncSeek + Send + Sync,
{
    async fn extract(&self) -> Result<Vec<u8>> {
        unimplemented!()
    }

    async fn replace(&mut self, data: &[u8]) -> Result<()> {
        unimplemented!()
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

#[derive(Debug)]
struct FullBox {
    ftyp: FileTypeBox,    // ftyp
    meta: MetaBox,        // meta
    free: Option<RawBox>, // free
    media: RawBox,        // mdat
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

#[derive(Debug)]
struct FileTypeBox {
    major_brand: String,
    minor_version: u32,
    compatible_brands: Vec<String>,
    data_ptr: Ptr,
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

#[derive(Debug)]
struct MetaBox {
    version: u8,
    flags: u32,
    boxes: HashMap<String, RawBox>,
    info_items: HashMap<String, ItemInfoBox>,
    full_ptr: Ptr,
    data_ptr: Ptr,
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
        let mut info_items = HashMap::new();

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

        // check required boxes
        if iinf_box.is_none() || iloc_box.is_none() {
            return Err(anyhow!("Missing required boxes"));
        }

        // make item info entry boxes

        Ok(Self {
            version,
            flags,
            boxes,
            full_ptr: raw_box.full_ptr.clone(),
            data_ptr: raw_box.data_ptr.clone(),
            info_items,
        })
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

#[derive(Debug)]
struct ItemInfoBox {
    version: u8,
    flags: u32,

    counts: u16,
    entries: Vec<ItemInfoEntry>,

    full_ptr: Ptr,
    data_ptr: Ptr,
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
            counts,
            entries,
            full_ptr: raw_box.full_ptr.clone(),
            data_ptr: raw_box.data_ptr.clone(),
        })
    }
}

#[derive(Debug)]
struct ItemInfoEntry {
    version: u8,
    flags: u32,

    item_id: u16,
    protection_index: u16,

    item_type: String,
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
        let item_id = u16::from_be_bytes([data[4], data[5]]);

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

#[derive(Debug)]
struct ItemLocationBox {
    version: u8,
    flags: u32,

    entries: HashMap<u32, ItemLocationEntry>,

    full_ptr: Ptr,
    data_ptr: Ptr,
}

#[derive(Debug)]
struct ItemLocationEntry {
    item_id: u32,
    construction_method: u16,
    data_reference_index: u16,
    base_offset: u64,
    extent_count: u16,
    extents: Vec<ItemLocationExtent>,
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
            extent_count,
            extents,
        })
    }
}

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
    use std::sync::Once;

    use tokio::fs;

    use crate::heic::FullBox;

    static INIT: Once = Once::new();

    fn init_logger() {
        INIT.call_once(|| {
            env_logger::builder()
                .filter_level(log::LevelFilter::Debug)
                .init();
        });
    }

    #[tokio::test]
    async fn read_full_box() {
        // init logger
        init_logger();

        // test files
        let files = vec![
            "sample/sample_by_iphone15-pro-max.heic",
            "sample/sample_by_hasselblad-x2d.heic",
        ];

        for file in files.into_iter() {
            let mut f = fs::File::open(file).await.expect("Failed to open file");

            let full_box = FullBox::from_reader(&mut f)
                .await
                .expect("failed to read full box");

            println!("--------{}\n{:#?}", file, full_box);
        }
    }
}
