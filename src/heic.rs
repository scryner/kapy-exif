use std::{
    collections::HashMap,
    io::{ErrorKind, SeekFrom},
};

use anyhow::Result;
use async_trait::async_trait;
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
            println!("--> {} / {:?}", box_type, raw_box);

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
    version: u32,
    boxes: HashMap<String, RawBox>,
    // info_items: HashMap<String, InfoItemBox>,
    data_ptr: Ptr,
}

impl MetaBox {
    async fn from_raw_box<R>(raw_box: &RawBox, r: &mut R) -> Result<Self>
    where
        R: AsyncRead + AsyncSeek + Send + Sync + Unpin,
    {
        let data = raw_box.data_ptr.read_data(r).await?;

        // read version
        let version = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let offset = raw_box.data_ptr.offset + 4;

        // read children boxes
        let mut cursor = std::io::Cursor::new(&data[4..]);

        let mut boxes = HashMap::new();
        loop {
            let mut raw_box = match RawBox::from_reader(&mut cursor).await? {
                Some(b) => b,
                None => break,
            };

            // advance
            raw_box.advance(&mut cursor).await?;
            let pos = cursor.seek(SeekFrom::Current(0)).await?;

            // adjust offset
            raw_box.full_ptr.offset += offset;
            raw_box.data_ptr.offset += offset;

            boxes.insert(raw_box.box_type.clone(), raw_box);
        }

        Ok(Self {
            version,
            boxes,
            data_ptr: raw_box.data_ptr.clone(),
        })
    }
}

#[derive(Debug)]
struct InfoItemBox {
    item_id: u32,
    item_type: String,
    version: u32,
    data_ptr: Ptr,
}

#[cfg(test)]
mod tests {
    use tokio::fs;

    use crate::heic::FullBox;

    #[tokio::test]
    async fn read_full_box() {
        // open file
        let mut f = fs::File::open("B0001612.HEIC")
            .await
            .expect("Failed to open file");

        let full_box = FullBox::from_reader(&mut f)
            .await
            .expect("failed to read full box");

        println!("{:#?}", full_box);
    }
}
